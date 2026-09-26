//! Computer name, elevation, processor architecture and fixed drives – all
//! through query-only Win32 calls.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    CheckTokenMembership, CreateWellKnownSid, GetTokenInformation, SECURITY_MAX_SID_SIZE, TOKEN_ELEVATION, TOKEN_QUERY,
    TokenElevation, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDriveStringsW, QueryDosDeviceW};
use windows_sys::Win32::System::SystemInformation::{
    ComputerNameDnsDomain, ComputerNameDnsFullyQualified, ComputerNameNetBIOS, GetComputerNameExW, GetNativeSystemInfo,
    PROCESSOR_ARCHITECTURE_AMD64, PROCESSOR_ARCHITECTURE_ARM64, PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// `GetDriveTypeW` results for fixed disks and network drives.
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;

/// Machine-wide variables used in commands (`%SystemRoot%`, `%ProgramFiles%`, …), from the
/// collector's own environment. User-specific ones (`%APPDATA%`, `%USERPROFILE%`) are left out:
/// they belong to whoever runs the collector, not to the user of an entry.
pub fn machine_variables() -> BTreeMap<String, String> {
    const NAMES: [&str; 13] = [
        "SystemRoot",
        "windir",
        "SystemDrive",
        "ProgramData",
        "ALLUSERSPROFILE",
        "PUBLIC",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
        "CommonProgramW6432",
        "ComSpec",
    ];
    NAMES
        .iter()
        .filter_map(|name| {
            let value = std::env::var(name).ok().filter(|value| !value.is_empty())?;
            Some((name.to_ascii_uppercase(), value))
        })
        .collect()
}

/// The drive letter of `C:\…` or `\\?\C:\…`.
pub fn drive_letter(path: &Path) -> Option<char> {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => Some(letter.to_ascii_uppercase()),
        _ => None,
    }
}

/// Whether a drive letter is mapped to a network share.
pub fn is_remote_drive(letter: char) -> bool {
    let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(Some(0)).collect();
    // SAFETY: `root` is a NUL-terminated root path such as "Z:\".
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}

pub enum NameKind {
    NetBios,
    Domain,
    FullyQualified,
}

/// The computer's NetBIOS name, DNS domain or FQDN (`None` if empty or unavailable).
pub fn computer_name(kind: NameKind) -> Option<String> {
    let format = match kind {
        NameKind::NetBios => ComputerNameNetBIOS,
        NameKind::Domain => ComputerNameDnsDomain,
        NameKind::FullyQualified => ComputerNameDnsFullyQualified,
    };
    let mut size = 0u32;
    // SAFETY: a null buffer with size 0 asks for the required size (the call fails with ERROR_MORE_DATA).
    unsafe { GetComputerNameExW(format, ptr::null_mut(), &mut size) };
    if size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; size as usize];
    // SAFETY: `buffer` holds `size` characters.
    if unsafe { GetComputerNameExW(format, buffer.as_mut_ptr(), &mut size) } == 0 {
        return None;
    }
    let name = String::from_utf16_lossy(&buffer[..(size as usize).min(buffer.len())]);
    (!name.is_empty()).then_some(name)
}

/// Whether the collector has administrator rights: the Administrators group is enabled in its
/// token (elevated administrator or SYSTEM). A filtered UAC token or a "basic user" token has the
/// group only as deny-only and counts as not elevated.
pub fn is_elevated() -> bool {
    let mut sid = [0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut size = SECURITY_MAX_SID_SIZE;
    // SAFETY: `sid` holds `size` bytes; no domain SID is needed for a built-in group.
    if unsafe { CreateWellKnownSid(WinBuiltinAdministratorsSid, ptr::null_mut(), sid.as_mut_ptr().cast(), &mut size) }
        == 0
    {
        return token_elevated();
    }
    let mut member = 0;
    // SAFETY: a null token checks the calling thread's (or process') token; `sid` is a valid SID.
    let ok = unsafe { CheckTokenMembership(ptr::null_mut(), sid.as_mut_ptr().cast(), &mut member) };
    if ok == 0 {
        return token_elevated();
    }
    member != 0
}

/// Fallback: the token's elevation flag.
fn token_elevated() -> bool {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: opens our own process token with query access only.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: `elevation` is a TOKEN_ELEVATION of the size we pass.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    // SAFETY: the token handle is ours and closed once.
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

/// Native processor architecture ("x64", "arm64", "x86").
pub fn native_architecture() -> Option<String> {
    // SAFETY: SYSTEM_INFO is plain data; GetNativeSystemInfo fills it completely.
    let info: SYSTEM_INFO = unsafe {
        let mut info = std::mem::zeroed::<SYSTEM_INFO>();
        GetNativeSystemInfo(&mut info);
        info
    };
    // SAFETY: the architecture member of the union is always initialised by GetNativeSystemInfo.
    let architecture = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture };
    match architecture {
        PROCESSOR_ARCHITECTURE_AMD64 => Some("x64".into()),
        PROCESSOR_ARCHITECTURE_ARM64 => Some("arm64".into()),
        PROCESSOR_ARCHITECTURE_INTEL => Some("x86".into()),
        _ => None,
    }
}

/// Root paths of fixed local drives (`C:\`, `D:\`, …). Network, removable and
/// optical drives are excluded, and so are SUBST drives, which would only
/// show folders of another drive a second time.
pub fn fixed_drives() -> Vec<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `buffer` holds the number of characters we pass.
    let len = unsafe { GetLogicalDriveStringsW(buffer.len() as u32, buffer.as_mut_ptr()) } as usize;
    if len == 0 || len > buffer.len() {
        return Vec::new();
    }
    buffer[..len]
        .split(|&unit| unit == 0)
        .filter(|root| !root.is_empty())
        .filter(|root| {
            let root_z: Vec<u16> = root.iter().copied().chain(Some(0)).collect();
            // SAFETY: `root_z` is a NUL-terminated root path such as "C:\".
            unsafe { GetDriveTypeW(root_z.as_ptr()) == DRIVE_FIXED }
        })
        .filter(|root| !is_substituted(root))
        .map(|root| PathBuf::from(String::from_utf16_lossy(root)))
        .collect()
}

/// SUBST drives map to `\??\<path>` instead of a volume device.
fn is_substituted(root: &[u16]) -> bool {
    let device: Vec<u16> = root.iter().copied().take_while(|&unit| unit != u16::from(b'\\')).chain(Some(0)).collect();
    let mut target = vec![0u16; 1024];
    // SAFETY: `device` is NUL-terminated ("C:"); `target` holds the number of characters we pass.
    let len = unsafe { QueryDosDeviceW(device.as_ptr(), target.as_mut_ptr(), target.len() as u32) } as usize;
    len > 0 && String::from_utf16_lossy(&target[..len.min(target.len())]).starts_with(r"\??\")
}
