use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use crate::{AppServerClient, Notification, ServerRequest};

#[path = "events/activation.rs"]
pub(super) mod activation;
use activation::await_activation;

pub(crate) const RESIDENT_NOTIFICATION_CAPACITY: usize = 1_000;
pub(crate) const RESIDENT_REQUEST_CAPACITY: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub enum ResidentNotificationEvent {
    Notification {
        generation: u64,
        notification: Notification,
    },
    Gap {
        generation: u64,
        skipped: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResidentServerRequestEvent {
    Request {
        generation: u64,
        request: ServerRequest,
    },
    Gap {
        generation: u64,
        skipped: u64,
    },
}

pub(crate) struct ResidentForwarders {
    pub(super) activation: watch::Sender<bool>,
    pub(super) generation_rx: watch::Receiver<u64>,
    pub(super) handles: Vec<JoinHandle<()>>,
}

impl ResidentForwarders {
    pub(crate) fn activate(&self) {
        let _ = self.activation.send(true);
    }

    pub(crate) async fn join(self) {
        drop(self.activation);
        for handle in self.handles {
            let _ = handle.await;
        }
    }
}

pub(crate) fn prepare_forwarders(
    client: &AppServerClient,
    generation: u64,
    notifications: broadcast::Sender<ResidentNotificationEvent>,
    server_requests: broadcast::Sender<ResidentServerRequestEvent>,
    generation_rx: watch::Receiver<u64>,
) -> ResidentForwarders {
    let (activation, activation_rx) = watch::channel(false);
    let death_generation_rx = generation_rx.clone();
    let notifications = spawn_notification_forwarder(
        client.subscribe_notifications(),
        generation,
        notifications,
        generation_rx.clone(),
        activation_rx.clone(),
    );
    let requests = spawn_request_forwarder(
        client.subscribe_server_requests(),
        generation,
        server_requests,
        generation_rx,
        activation_rx,
    );
    ResidentForwarders {
        activation,
        generation_rx: death_generation_rx,
        handles: vec![notifications, requests],
    }
}

fn spawn_notification_forwarder(
    mut source: broadcast::Receiver<Notification>,
    generation: u64,
    target: broadcast::Sender<ResidentNotificationEvent>,
    mut generation_rx: watch::Receiver<u64>,
    activation_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !await_activation(activation_rx).await {
            return;
        }
        loop {
            if *generation_rx.borrow() != generation {
                drain_notifications(&mut source, generation, &target);
                break;
            }
            tokio::select! {
                changed = generation_rx.changed() => {
                    if changed.is_err() || *generation_rx.borrow() != generation {
                        drain_notifications(&mut source, generation, &target);
                        break;
                    }
                }
                item = source.recv() => if !forward_notification(item, generation, &target) {
                    break;
                }
            }
        }
    })
}

fn spawn_request_forwarder(
    mut source: broadcast::Receiver<ServerRequest>,
    generation: u64,
    target: broadcast::Sender<ResidentServerRequestEvent>,
    mut generation_rx: watch::Receiver<u64>,
    activation_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !await_activation(activation_rx).await {
            return;
        }
        loop {
            if *generation_rx.borrow() != generation {
                drain_requests(&mut source, generation, &target);
                break;
            }
            tokio::select! {
                changed = generation_rx.changed() => {
                    if changed.is_err() || *generation_rx.borrow() != generation {
                        drain_requests(&mut source, generation, &target);
                        break;
                    }
                }
                item = source.recv() => if !forward_request(item, generation, &target) {
                    break;
                }
            }
        }
    })
}

fn drain_notifications(
    source: &mut broadcast::Receiver<Notification>,
    generation: u64,
    target: &broadcast::Sender<ResidentNotificationEvent>,
) {
    loop {
        match source.try_recv() {
            Ok(notification) => {
                let _ = target.send(ResidentNotificationEvent::Notification {
                    generation,
                    notification,
                });
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                let _ = target.send(ResidentNotificationEvent::Gap {
                    generation,
                    skipped,
                });
            }
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
        }
    }
}

fn drain_requests(
    source: &mut broadcast::Receiver<ServerRequest>,
    generation: u64,
    target: &broadcast::Sender<ResidentServerRequestEvent>,
) {
    loop {
        match source.try_recv() {
            Ok(request) => {
                let _ = target.send(ResidentServerRequestEvent::Request {
                    generation,
                    request,
                });
            }
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                let _ = target.send(ResidentServerRequestEvent::Gap {
                    generation,
                    skipped,
                });
            }
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
        }
    }
}

fn forward_notification(
    item: Result<Notification, broadcast::error::RecvError>,
    generation: u64,
    target: &broadcast::Sender<ResidentNotificationEvent>,
) -> bool {
    match item {
        Ok(notification) => {
            let _ = target.send(ResidentNotificationEvent::Notification {
                generation,
                notification,
            });
            true
        }
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            let _ = target.send(ResidentNotificationEvent::Gap {
                generation,
                skipped,
            });
            true
        }
        Err(broadcast::error::RecvError::Closed) => false,
    }
}

fn forward_request(
    item: Result<ServerRequest, broadcast::error::RecvError>,
    generation: u64,
    target: &broadcast::Sender<ResidentServerRequestEvent>,
) -> bool {
    match item {
        Ok(request) => {
            let _ = target.send(ResidentServerRequestEvent::Request {
                generation,
                request,
            });
            true
        }
        Err(broadcast::error::RecvError::Lagged(skipped)) => {
            let _ = target.send(ResidentServerRequestEvent::Gap {
                generation,
                skipped,
            });
            true
        }
        Err(broadcast::error::RecvError::Closed) => false,
    }
}

#[cfg(test)]
mod tests;
