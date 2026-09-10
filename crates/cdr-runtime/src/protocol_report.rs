//! Protocol registration is configuration evidence, not UI activation evidence.
#[cfg(windows)]
pub(super) fn report() -> String {
    use cdr_windows_native::protocol::{ProtocolRegistration, registration};
    let status = match registration("codex") {
        Ok(ProtocolRegistration::KeyMissing) => "현재 사용자 등록 키 없음".into(),
        Ok(ProtocolRegistration::UrlMarkerMissing) => {
            "등록 키는 있으나 URL Protocol 표식 없음".into()
        }
        Ok(ProtocolRegistration::UrlMarkerPresent) => {
            "등록 키·URL Protocol 문자열 표식 있음".into()
        }
        Ok(ProtocolRegistration::UnexpectedMarkerType(kind)) => {
            format!("URL Protocol 표식 형식 미확인: {kind}")
        }
        Err(error) => format!("조회 실패: {error}"),
    };
    format!(
        "codex_protocol_registration: {status} · HKCR metadata only · 앱 열기 성공을 뜻하지 않음"
    )
}

#[cfg(not(windows))]
pub(super) fn report() -> String {
    "codex_protocol_registration: 이 OS의 등록 검사 미구현 · 앱 열기 성공을 뜻하지 않음".into()
}
