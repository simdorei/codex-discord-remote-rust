use std::env;
use std::path::{Path, PathBuf};

use crate::computer::{ComputerError, ComputerIdentity};

const SECURITY_MARKERS: &[&str] = &[
    "1password",
    "additional verification",
    "authentication",
    "authenticator",
    "bitwarden",
    "chatgpt",
    "chrome remote desktop",
    "codex",
    "credential manager",
    "developer tools",
    "devtools",
    "enter your password",
    "enter code",
    "google accounts",
    "lastpass",
    "one-time code",
    "one-time password",
    "passcode",
    "password",
    "privacy & security",
    "sign in",
    "sign-in",
    "two-step verification",
    "2-step verification",
    "verification",
    "verification code",
    "verify it's you",
    "confirm your identity",
    "account recovery",
    "recovery code",
    "security check",
    "security code",
    "access code",
    "windows security",
    "로그인",
    "본인인증",
    "비밀번호",
    "암호 입력",
    "인증번호",
    "보안 코드",
    "본인 확인",
    "계정 복구",
    "추가인증",
    "개인 정보 및 보안",
];
const PROTECTED_TITLES: &[&str] = &[
    "open",
    "page setup",
    "print",
    "run",
    "save as",
    "열기",
    "실행",
    "인쇄",
    "다른 이름으로 저장",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectApplication {
    Chrome,
    Notepad,
}

impl ProjectApplication {
    pub fn process_name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome.exe",
            Self::Notepad => "notepad.exe",
        }
    }
}

pub(super) fn project_application(path: &Path) -> Result<ProjectApplication, ComputerError> {
    let candidate = normalized(path)?;
    let chrome_suffix = Path::new("Google/Chrome/Application/chrome.exe");
    for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(base) = env::var_os(variable).map(PathBuf::from)
            && candidate == normalized(&base.join(chrome_suffix))?
        {
            return Ok(ProjectApplication::Chrome);
        }
    }
    if let Some(root) = env::var_os("SYSTEMROOT").map(PathBuf::from)
        && candidate == normalized(&root.join("System32/notepad.exe"))?
    {
        return Ok(ProjectApplication::Notepad);
    }
    if let Some(program_files) = env::var_os("PROGRAMFILES").map(PathBuf::from) {
        let apps = normalized(&program_files.join("WindowsApps"))?;
        if let Ok(relative) = candidate.strip_prefix(apps)
            && relative.components().count() >= 2
            && relative.components().next().is_some_and(|part| {
                part.as_os_str()
                    .to_string_lossy()
                    .starts_with("microsoft.windowsnotepad_")
            })
            && relative
                .file_name()
                .is_some_and(|name| name == "notepad.exe")
        {
            return Ok(ProjectApplication::Notepad);
        }
    }
    Err(ComputerError::Platform(
        "This protected or unapproved application cannot be controlled.".into(),
    ))
}

pub(super) fn safe_project_title(
    application: ProjectApplication,
    title: &str,
) -> Result<String, ComputerError> {
    let normalized = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if PROTECTED_TITLES.contains(&normalized.as_str())
        || SECURITY_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
    {
        return Err(ComputerError::Platform(
            "This security, sign-in, or protected window requires direct user control.".into(),
        ));
    }
    Ok(match application {
        ProjectApplication::Chrome => "Google Chrome".into(),
        ProjectApplication::Notepad => "Notepad".into(),
    })
}

pub(super) fn require_notepad(identity: &ComputerIdentity) -> Result<(), ComputerError> {
    if project_application(Path::new(&identity.process_path))? == ProjectApplication::Notepad {
        Ok(())
    } else {
        Err(ComputerError::Platform(
            "Chrome interactions require direct user control so private page content stays private."
                .into(),
        ))
    }
}

pub(super) fn require_project_keys(keys: &[String]) -> Result<(), ComputerError> {
    let normalized = keys
        .iter()
        .map(|key| key.trim().to_ascii_uppercase().replace(' ', ""))
        .collect::<Vec<_>>();
    let allowed_single = [
        "BACKSPACE",
        "DELETE",
        "DOWN",
        "END",
        "ENTER",
        "ESC",
        "HOME",
        "LEFT",
        "PAGEDOWN",
        "PAGEUP",
        "RIGHT",
        "SPACE",
        "TAB",
        "UP",
    ];
    let single = normalized.len() == 1 && allowed_single.contains(&normalized[0].as_str());
    let control = normalized.len() == 2
        && normalized.iter().any(|key| key == "CTRL")
        && normalized
            .iter()
            .any(|key| ["A", "C", "X", "Z"].contains(&key.as_str()));
    if single || control {
        Ok(())
    } else {
        Err(ComputerError::Platform(
            "This protected keyboard shortcut is not available.".into(),
        ))
    }
}

fn normalized(path: &Path) -> Result<PathBuf, ComputerError> {
    if !path.is_absolute() {
        return Err(ComputerError::Platform(
            "Windows could not verify the application path.".into(),
        ));
    }
    Ok(PathBuf::from(path.to_string_lossy().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::{ProjectApplication, require_project_keys, safe_project_title};

    #[test]
    fn protected_titles_are_never_exposed_by_project_mode() {
        assert!(safe_project_title(ProjectApplication::Chrome, "Sign in - Google").is_err());
        assert_eq!(
            safe_project_title(ProjectApplication::Notepad, "memo.txt").unwrap(),
            "Notepad"
        );
    }

    #[test]
    fn project_keys_are_narrowly_allowlisted() {
        assert!(require_project_keys(&["CTRL".into(), "A".into()]).is_ok());
        assert!(require_project_keys(&["ALT".into(), "F4".into()]).is_err());
    }
}
