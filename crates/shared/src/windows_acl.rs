// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Creating files, folders and named pipes that other local users can't open. Windows
//! gives new objects an inherited or default DACL, which lets every user read
//! a file under `%ProgramData%` and open a named pipe for reading; these
//! helpers set an explicit, protected one instead.

use std::{
    ffi::c_void,
    io,
    os::windows::{
        ffi::OsStrExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    },
    path::Path,
    ptr,
};

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        ACL,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT, SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        GetTokenInformation, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
        TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateDirectoryW, CreateFileW,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_WRITE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_DATA,
        GetFileInformationByHandle, OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
        WRITE_OWNER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

/// Who may open an object, as a protected DACL (`P`: nothing inherited).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// `LocalSystem` and the Administrators group only: a service's
    /// configuration, which an administrator writes and the service reads.
    SystemAndAdministrators,
    /// `LocalSystem`, Administrators and the object's owner (the user that
    /// created it): a pipe between one user's own processes.
    SystemAdministratorsAndOwner,
}

impl Access {
    const fn sddl(self) -> &'static str {
        match self {
            Self::SystemAndAdministrators => "D:P(A;;GA;;;SY)(A;;GA;;;BA)",
            Self::SystemAdministratorsAndOwner => "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)",
        }
    }

    /// The same DACL for a folder, inherited by everything created in it
    /// (`OICI`: object and container inherit).
    const fn inheritable_sddl(self) -> &'static str {
        match self {
            // Owned by the Administrators group rather than by whichever
            // account made it: a folder an administrator creates is then
            // one `LocalSystem` trusts too (see `is_trusted_owner`).
            Self::SystemAndAdministrators => "O:BAD:P(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)",
            Self::SystemAdministratorsAndOwner => {
                "D:P(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)(A;OICI;GA;;;OW)"
            }
        }
    }
}

/// A security descriptor built from [`Access`], freed on drop.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Descriptor {
    fn new(access: Access) -> io::Result<Self> {
        Self::from_sddl(access.sddl())
    }

    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let sddl: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated UTF-16 that outlives the call, and
        // `descriptor` is a valid out-pointer. On success the function stores a
        // LocalAlloc'd descriptor there, which `Drop` frees.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    /// Attributes pointing at this descriptor; valid while `self` lives.
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from ConvertStringSecurityDescriptorToSecurityDescriptorW,
        // which documents LocalFree as the way to release it, and is freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// Creates `path` as a new file only `access` may open, replacing any file
/// already there (whose DACL would otherwise be kept). The DACL is part of
/// the creation itself, so there is no moment another user could open the
/// file, and nobody else may open it while the returned handle is held.
///
/// # Errors
///
/// Fails if the old file can't be removed or the new one can't be created.
pub async fn create_private_file(path: &Path, access: Access) -> io::Result<std::fs::File> {
    if let Err(e) = tokio::fs::remove_file(path).await
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e);
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    tokio::task::spawn_blocking(move || create_new_file(&wide, access))
        .await
        .map_err(io::Error::other)?
}

/// `CreateFileW` for a file that must not exist yet; `wide` is its
/// NUL-terminated path.
fn create_new_file(wide: &[u16], access: Access) -> io::Result<std::fs::File> {
    let descriptor = Descriptor::new(access)?;
    let attributes = descriptor.attributes();
    // SAFETY: `wide` is a NUL-terminated path and `attributes` points at a
    // descriptor `descriptor` keeps alive; both outlive the call. Share mode 0
    // keeps the file exclusive to this handle.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            0,
            &raw const attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a file handle CreateFileW just opened, owned by
    // nothing else; the File takes over closing it.
    Ok(unsafe { std::fs::File::from_raw_handle(handle) })
}

/// Creates the folder `path`, whose parent must exist, so that only `access`
/// may open it and everything created in it. A new folder gets the DACL as
/// part of its creation; an existing one has its DACL replaced, and Windows
/// replaces what its contents inherited from the old one too. Permissions
/// set on a file directly, rather than inherited, are not replaced; a file
/// that is written to again should be opened with [`open_private_log`].
///
/// An existing folder is trusted only if `LocalSystem`, the Administrators
/// group or this process's own user owns it, and it is not a junction or
/// link: another user could create one under `%ProgramData%` in advance to
/// read what is written there, or to have its DACL set on a folder of their
/// choosing.
///
/// # Errors
///
/// Fails if the folder can't be created, an existing one isn't trusted, or
/// its DACL can't be set.
pub async fn create_private_dir(path: &Path, access: Access) -> io::Result<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    tokio::task::spawn_blocking(move || create_dir(&wide, access))
        .await
        .map_err(io::Error::other)?
}

/// [`create_private_dir`] for the NUL-terminated path `wide`.
fn create_dir(wide: &[u16], access: Access) -> io::Result<()> {
    let descriptor = Descriptor::from_sddl(access.inheritable_sddl())?;
    let attributes = descriptor.attributes();
    // SAFETY: `wide` is a NUL-terminated path and `attributes` points at a
    // descriptor `descriptor` keeps alive; both outlive the call.
    if unsafe { CreateDirectoryW(wide.as_ptr(), &raw const attributes) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != i32::try_from(ERROR_ALREADY_EXISTS).ok() {
        return Err(error);
    }
    let folder = open_trusted(wide, FILE_FLAG_BACKUP_SEMANTICS, Kind::Folder)?;
    set_security(&folder, &descriptor)
}

/// Sets the owner, if `descriptor` names one, and the protected DACL of
/// `descriptor` on the object `handle` refers to, which must be open with
/// `WRITE_DAC` (and `WRITE_OWNER` for an owner). Setting them through the
/// handle reaches the object that was checked, whatever its path names by
/// now. The group and SACL are left as they are.
fn set_security(handle: &OwnedHandle, descriptor: &Descriptor) -> io::Result<()> {
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    // SAFETY: `descriptor.0` is a valid descriptor and the out-pointers are
    // valid; the DACL it returns points into the descriptor, which outlives
    // its use below.
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor.0,
            &raw mut present,
            &raw mut dacl,
            &raw mut defaulted,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut owner: PSID = ptr::null_mut();
    // SAFETY: `descriptor.0` is a valid descriptor and the out-pointers are
    // valid; the owner it returns, if any, points into the descriptor.
    if unsafe { GetSecurityDescriptorOwner(descriptor.0, &raw mut owner, &raw mut defaulted) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut information = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    if !owner.is_null() {
        information |= OWNER_SECURITY_INFORMATION;
    }
    // SAFETY: `handle` is open with the rights the caller is required to
    // open it with, and `owner` and `dacl` point into `descriptor`, which
    // outlives the call.
    let status = unsafe {
        SetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            information,
            owner,
            ptr::null_mut(),
            dacl,
            ptr::null(),
        )
    };
    win32_result(status)
}

/// Opens the file `path` for appending, creating it if it doesn't exist,
/// so that only `access` may open it. A new file gets the DACL as part of
/// its creation. An existing one keeps what was written to it, unlike with
/// [`create_private_file`], and has its DACL replaced, inherited and
/// directly set permissions alike, if it is trusted the way an existing
/// folder is (see [`create_private_dir`]) and has no other hard link.
///
/// # Errors
///
/// Fails if the file can't be opened, an existing one isn't trusted, or its
/// DACL can't be set.
pub async fn open_private_log(path: &Path, access: Access) -> io::Result<std::fs::File> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    tokio::task::spawn_blocking(move || open_log(&wide, access))
        .await
        .map_err(io::Error::other)?
}

/// [`open_private_log`] for the NUL-terminated path `wide`.
fn open_log(wide: &[u16], access: Access) -> io::Result<std::fs::File> {
    let descriptor = Descriptor::new(access)?;
    let attributes = descriptor.attributes();
    // SAFETY: `wide` is a NUL-terminated path and `attributes` points at a
    // descriptor `descriptor` keeps alive; both outlive the call. Leaving
    // out FILE_WRITE_DATA makes every write an append. A link is opened
    // itself rather than followed, so the check below can refuse it.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            (FILE_GENERIC_WRITE & !FILE_WRITE_DATA) | WRITE_DAC,
            FILE_SHARE_READ,
            &raw const attributes,
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    let existed =
        io::Error::last_os_error().raw_os_error() == i32::try_from(ERROR_ALREADY_EXISTS).ok();
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` was just opened and is owned by nothing else.
    let file = unsafe { OwnedHandle::from_raw_handle(handle) };
    if existed {
        trust_check(&file, wide, Kind::File)?;
        set_security(&file, &descriptor)?;
    }
    Ok(std::fs::File::from(file))
}

/// What an existing object must be to be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Folder,
    File,
}

/// Opens the existing folder or file at the NUL-terminated path `wide` to
/// set its owner and DACL, and checks it may be trusted (see
/// [`trust_check`]). `flags` are added to the open's flags.
fn open_trusted(wide: &[u16], flags: u32, kind: Kind) -> io::Result<OwnedHandle> {
    // SAFETY: `wide` is a NUL-terminated path that outlives the call.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            READ_CONTROL | WRITE_DAC | WRITE_OWNER,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            flags | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` was just opened and is owned by nothing else.
    let object = unsafe { OwnedHandle::from_raw_handle(handle) };
    trust_check(&object, wide, kind)?;
    Ok(object)
}

/// Checks that the object `handle` refers to, opened from the
/// NUL-terminated path `wide` without following a junction or link and
/// with `READ_CONTROL`, may be trusted: it is a `kind`, not a junction or
/// link, a file has no other hard link to it, and `LocalSystem`, the
/// Administrators group or this process's own user owns it. Another user
/// could create one in advance to read what is written there, or to have
/// a DACL set on an object of their choosing.
fn trust_check(handle: &OwnedHandle, wide: &[u16], kind: Kind) -> io::Result<()> {
    let path = String::from_utf16_lossy(wide.strip_suffix(&[0]).unwrap_or(wide));

    // SAFETY: an all-zero BY_HANDLE_FILE_INFORMATION is a valid value.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is open and `info` is a valid out-pointer.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &raw mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::other(format!(
            "{path} is a junction or link; remove it and try again"
        )));
    }
    let is_folder = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    match kind {
        Kind::Folder if !is_folder => {
            return Err(io::Error::other(format!("{path} is not a folder")));
        }
        Kind::File if is_folder => {
            return Err(io::Error::other(format!("{path} is a folder, not a file")));
        }
        Kind::File if info.nNumberOfLinks != 1 => {
            return Err(io::Error::other(format!(
                "{path} has another hard link to it; remove it and try again"
            )));
        }
        Kind::Folder | Kind::File => {}
    }

    let mut owner: PSID = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `handle` is open with READ_CONTROL and the out-pointers are
    // valid. `owner` points into `descriptor`, freed by the guard below.
    let status = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    win32_result(status)?;
    let descriptor = Descriptor(descriptor);
    if !is_trusted_owner(owner)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{path} is owned by another user, so what is written there could be read by them; \
                 remove it and try again"
            ),
        ));
    }
    drop(descriptor);
    Ok(())
}

/// Whether `owner` is `LocalSystem`, the Administrators group or this
/// process's own user.
fn is_trusted_owner(owner: PSID) -> io::Result<bool> {
    // SAFETY: `owner` is a valid SID for the duration of the call.
    let well_known = unsafe {
        IsWellKnownSid(owner, WinLocalSystemSid) != 0
            || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
    };
    if well_known {
        return Ok(true);
    }
    let user = current_user()?;
    // SAFETY: both SIDs are valid; `user` keeps its SID alive.
    Ok(unsafe { EqualSid(owner, user.sid()) } != 0)
}

/// This process's user, as returned by `GetTokenInformation`.
struct TokenUserInfo(Vec<u64>);

impl TokenUserInfo {
    fn sid(&self) -> PSID {
        // SAFETY: the buffer holds a TOKEN_USER written by
        // GetTokenInformation, suitably aligned by its u64 elements.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn current_user() -> io::Result<TokenUserInfo> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no
    // closing, and `token` is a valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `token` was just opened and is owned by nothing else.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut needed = 0;
    // SAFETY: a null buffer of size 0 asks only for the size needed.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &raw mut needed,
        );
    }
    let words = usize::try_from(needed).unwrap_or(0).div_ceil(8);
    let mut buffer = vec![0u64; words.max(1)];
    let size = u32::try_from(buffer.len().saturating_mul(8)).unwrap_or(u32::MAX);
    // SAFETY: `buffer` holds at least `size` writable bytes.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &raw mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(TokenUserInfo(buffer))
}

/// A Win32 status code as a result.
fn win32_result(status: u32) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(
            i32::try_from(status).unwrap_or(i32::MAX),
        ))
    }
}

/// Creates a named pipe server instance that only `access` may open.
///
/// # Errors
///
/// Fails if the descriptor can't be built or the pipe can't be created.
pub fn create_private_pipe(
    options: &ServerOptions,
    name: &str,
    access: Access,
) -> io::Result<NamedPipeServer> {
    let descriptor = Descriptor::new(access)?;
    let mut attributes = descriptor.attributes();
    let attributes: *mut c_void = (&raw mut attributes).cast();
    // SAFETY: `attributes` points at a SECURITY_ATTRIBUTES whose descriptor
    // `descriptor` keeps alive until after the pipe is created.
    unsafe { options.create_with_security_attributes_raw(name, attributes) }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
        },
        PSID,
    };

    use super::*;

    #[test]
    fn every_access_level_builds_a_descriptor() {
        for access in [
            Access::SystemAndAdministrators,
            Access::SystemAdministratorsAndOwner,
        ] {
            Descriptor::new(access).unwrap();
            Descriptor::from_sddl(access.inheritable_sddl()).unwrap();
        }
    }

    /// The DACL of `path` in SDDL.
    fn dacl_of(path: &Path) -> String {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let (mut owner, mut group): (PSID, PSID) = (ptr::null_mut(), ptr::null_mut());
        let (mut dacl, mut sacl): (*mut ACL, *mut ACL) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: `wide` is NUL-terminated and every out-pointer is valid.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                &raw mut owner,
                &raw mut group,
                &raw mut dacl,
                &raw mut sacl,
                &raw mut descriptor,
            )
        };
        assert_eq!(status, ERROR_SUCCESS);
        let descriptor = Descriptor(descriptor);
        let mut text: *mut u16 = ptr::null_mut();
        let mut len = 0;
        // SAFETY: `descriptor.0` is valid and the out-pointers are valid.
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &raw mut text,
                &raw mut len,
            )
        };
        assert_ne!(converted, 0);
        // SAFETY: on success `text` holds `len` UTF-16 units, LocalAlloc'd.
        let sddl = String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(text, usize::try_from(len).unwrap())
        });
        // SAFETY: `text` came from the conversion above and is freed once.
        unsafe { LocalFree(text.cast()) };
        sddl.trim_end_matches('\0').to_owned()
    }

    /// The trustees an SDDL DACL grants anything to.
    fn trustees(sddl: &str) -> Vec<String> {
        sddl.split('(')
            .skip(1)
            .filter_map(|ace| ace.trim_end_matches(')').rsplit(';').next())
            .map(str::to_owned)
            .collect()
    }

    /// Wine ignores the DACL a folder is created or set with, so this holds
    /// only on Windows. The owner keeps access so the test can clean up.
    #[tokio::test]
    async fn a_private_dir_restricts_what_is_created_in_it_and_what_was_there() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join("fresh");
        let existing = root.path().join("existing");
        tokio::fs::create_dir(&existing).await.unwrap();
        tokio::fs::write(existing.join("old.log"), b"old")
            .await
            .unwrap();
        let mut inherited = trustees(&dacl_of(&existing.join("old.log")));
        inherited.sort();
        inherited.dedup();
        assert_ne!(inherited, ["BA", "OW", "SY"], "already restricted");

        create_private_dir(&fresh, Access::SystemAdministratorsAndOwner)
            .await
            .unwrap();
        create_private_dir(&existing, Access::SystemAdministratorsAndOwner)
            .await
            .unwrap();
        tokio::fs::write(fresh.join("new.log"), b"new")
            .await
            .unwrap();

        for path in [
            fresh.clone(),
            fresh.join("new.log"),
            existing.clone(),
            existing.join("old.log"),
        ] {
            let mut granted = trustees(&dacl_of(&path));
            granted.sort();
            granted.dedup();
            assert_eq!(
                granted,
                ["BA", "OW", "SY"],
                "{}: {}",
                path.display(),
                dacl_of(&path)
            );
        }
    }

    /// The sorted, distinct trustees `path`'s DACL grants access to.
    fn granted(path: &Path) -> Vec<String> {
        let mut granted = trustees(&dacl_of(path));
        granted.sort();
        granted.dedup();
        granted
    }

    /// Wine ignores DACLs, so this holds only on Windows.
    #[tokio::test]
    async fn a_private_log_keeps_its_contents_and_loses_other_grants() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join("fresh.log");
        let existing = root.path().join("existing.log");
        tokio::fs::write(&existing, b"old\n").await.unwrap();
        let everyone = std::process::Command::new("icacls")
            .arg(&existing)
            .arg("/grant")
            .arg("*S-1-1-0:(R)")
            .output()
            .unwrap();
        assert!(everyone.status.success(), "{everyone:?}");
        assert!(granted(&existing).contains(&"WD".to_owned()));

        for path in [&fresh, &existing] {
            let mut log = open_private_log(path, Access::SystemAdministratorsAndOwner)
                .await
                .unwrap();
            std::io::Write::write_all(&mut log, b"new\n").unwrap();
        }

        assert_eq!(tokio::fs::read(&fresh).await.unwrap(), b"new\n");
        assert_eq!(tokio::fs::read(&existing).await.unwrap(), b"old\nnew\n");
        for path in [&fresh, &existing] {
            assert_eq!(
                granted(path),
                ["BA", "OW", "SY"],
                "{}: {}",
                path.display(),
                dacl_of(path)
            );
        }
    }

    /// A second hard link could name a file elsewhere, whose DACL would
    /// then be replaced.
    #[tokio::test]
    async fn a_private_log_refuses_a_hard_link() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original.log");
        let link = root.path().join("link.log");
        tokio::fs::write(&original, b"old").await.unwrap();
        tokio::fs::hard_link(&original, &link).await.unwrap();

        let refused = open_private_log(&link, Access::SystemAndAdministrators)
            .await
            .unwrap_err();

        assert!(refused.to_string().contains("hard link"), "{refused}");
    }

    /// A junction is refused, and the folder it points at keeps its DACL.
    #[tokio::test]
    async fn a_private_dir_refuses_a_junction() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let junction = root.path().join("junction");
        tokio::fs::create_dir(&target).await.unwrap();
        let made = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let before = dacl_of(&target);

        let error = create_private_dir(&junction, Access::SystemAndAdministrators)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("junction or link"), "{error}");
        assert_eq!(dacl_of(&target), before);
    }

    /// Whether `path` is owned by the Administrators group.
    fn owned_by_administrators(path: &Path) -> bool {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        let mut owner: PSID = ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and the out-pointers are valid.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &raw mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        assert_eq!(status, ERROR_SUCCESS);
        let descriptor = Descriptor(descriptor);
        // SAFETY: `owner` points into `descriptor`, which is still alive.
        let administrators = unsafe { IsWellKnownSid(owner, WinBuiltinAdministratorsSid) } != 0;
        drop(descriptor);
        administrators
    }

    /// `service install` runs as an administrator and the service as
    /// `LocalSystem`; a folder either made must be one the other trusts, so
    /// it belongs to the Administrators group whoever made it. Wine ignores
    /// the owner a folder is created with, so this holds only on Windows.
    #[tokio::test]
    async fn a_service_dir_is_owned_by_the_administrators_group() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join("fresh");
        let existing = root.path().join("existing");
        tokio::fs::create_dir(&existing).await.unwrap();

        create_private_dir(&fresh, Access::SystemAndAdministrators)
            .await
            .unwrap();
        create_private_dir(&existing, Access::SystemAndAdministrators)
            .await
            .unwrap();

        assert!(owned_by_administrators(&fresh));
        assert!(owned_by_administrators(&existing));
        // A folder owned by Administrators is trusted the next time too.
        create_private_dir(&fresh, Access::SystemAndAdministrators)
            .await
            .unwrap();
    }

    #[test]
    fn this_process_trusts_a_folder_it_owns() {
        let user = current_user().unwrap();
        assert!(is_trusted_owner(user.sid()).unwrap());
    }

    #[tokio::test]
    async fn a_private_file_replaces_an_existing_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        tokio::fs::write(&path, b"old").await.unwrap();

        let file = create_private_file(&path, Access::SystemAdministratorsAndOwner)
            .await
            .unwrap();
        drop(file);

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"");
    }
}
