use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use cdr_core::deadline::RequestBudget;
use futures_util::StreamExt;
use reqwest::header::LOCATION;
use tokio::sync::watch;
use url::Url;

use super::{ImageError, MAX_IMAGE_BYTES};

const MAX_REDIRECTS: usize = 5;

pub async fn download(
    value: &str,
    mut cancelled: watch::Receiver<bool>,
    budget: &RequestBudget,
) -> Result<Vec<u8>, ImageError> {
    let mut current = validate_url(value)?;
    for redirect_count in 0..=MAX_REDIRECTS {
        let remaining = budget.remaining(None).map_err(|_| ImageError::TimedOut)?;
        let request = request_once(&current, &mut cancelled);
        let response = tokio::time::timeout(remaining, request)
            .await
            .map_err(|_| ImageError::TimedOut)??;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(network("image URL redirected too many times"));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or_else(|| network("image redirect did not include a location"))?
                .to_str()
                .map_err(|_| network("image redirect location is invalid"))?;
            current = validate_url(
                current
                    .join(location)
                    .map_err(|_| network("image redirect location is invalid"))?
                    .as_str(),
            )?;
            continue;
        }
        let response = response
            .error_for_status()
            .map_err(|error| network(&format!("image download failed: {error}")))?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_IMAGE_BYTES as u64)
        {
            return Err(network(&format!("image exceeds {MAX_IMAGE_BYTES} bytes")));
        }
        return read_body(response, &mut cancelled).await;
    }
    unreachable!()
}

async fn request_once(
    url: &Url,
    cancelled: &mut watch::Receiver<bool>,
) -> Result<reqwest::Response, ImageError> {
    let host = url
        .host_str()
        .ok_or_else(|| network("image URL has no hostname"))?;
    let addresses = resolve_public(host).await?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(30))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|error| network(&format!("image client setup failed: {error}")))?;
    tokio::select! {
        response = client.get(url.clone()).send() => {
            response.map_err(|error| network(&format!("image download failed: {error}")))
        }
        () = wait_for_cancellation(cancelled) => Err(ImageError::Cancelled),
    }
}

async fn read_body(
    response: reqwest::Response,
    cancelled: &mut watch::Receiver<bool>,
) -> Result<Vec<u8>, ImageError> {
    let mut content = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            chunk = stream.next() => chunk,
            () = wait_for_cancellation(cancelled) => return Err(ImageError::Cancelled),
        };
        let Some(chunk) = chunk else {
            return Ok(content);
        };
        let chunk = chunk.map_err(|error| network(&format!("image download failed: {error}")))?;
        if content.len() + chunk.len() > MAX_IMAGE_BYTES {
            return Err(network(&format!("image exceeds {MAX_IMAGE_BYTES} bytes")));
        }
        content.extend_from_slice(&chunk);
    }
}

fn validate_url(value: &str) -> Result<Url, ImageError> {
    let url = Url::parse(value).map_err(|_| network("image URL is invalid"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
    {
        return Err(network(
            "image URL must be public HTTPS on the standard port",
        ));
    }
    if let Some(host) = url.host_str()
        && let Ok(address) = host.parse::<IpAddr>()
        && !is_public(address)
    {
        return Err(network("image URL must use the public network"));
    }
    Ok(url)
}

async fn resolve_public(host: &str) -> Result<Vec<SocketAddr>, ImageError> {
    let mut addresses = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| network("image URL hostname could not be resolved"))?
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public(address.ip())) {
        return Err(network("image URL must use the public network"));
    }
    Ok(addresses)
}

fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => public_v4(address),
        IpAddr::V6(address) => public_v6(address),
    }
}

fn public_v4(address: Ipv4Addr) -> bool {
    let [a, b, _, _] = address.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && matches!(b, 0 | 168))
        || (a == 198 && matches!(b, 18 | 19 | 51))
        || (a == 203 && b == 0)
        || a >= 224)
}

fn public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return public_v4(mapped);
    }
    let first = address.segments()[0];
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80
        || first & 0xffc0 == 0xfec0
        || (address.segments()[0] == 0x2001 && address.segments()[1] == 0x0db8))
}

async fn wait_for_cancellation(cancelled: &mut watch::Receiver<bool>) {
    loop {
        if *cancelled.borrow() {
            return;
        }
        if cancelled.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn network(message: &str) -> ImageError {
    ImageError::Network(message.to_owned())
}
