// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Exercises [`KeychainStore`] against the real OS keychain: Keychain
//! Services on macOS, the Secret Service on Linux (in CI, an unlocked
//! `gnome-keyring` inside a throwaway D-Bus session). Every run files its
//! entries under a unique service name and removes them again, so it never
//! touches the desktop app's real secrets.

use desktop_core::secrets::{KeychainStore, Secret, SecretName, SecretStore, load_or_create};

#[cfg(test)]
fn test_store() -> KeychainStore {
    let suffix = Secret::generate();
    KeychainStore::with_service(format!(
        "assimilate-desktop-test-{}",
        suffix.expose().get(..12).unwrap()
    ))
}

#[test]
#[ignore = "requires an unlocked OS keychain"]
fn keychain_stores_reads_overwrites_and_deletes_secrets() {
    let store = test_store();
    let name = SecretName::AdminPassword;

    store.delete(name).unwrap();
    assert_eq!(
        store.get(name).unwrap(),
        None,
        "a fresh service holds nothing"
    );

    let first = load_or_create(&store, name).unwrap();
    assert_eq!(store.get(name).unwrap(), Some(first.clone()));
    assert_eq!(load_or_create(&store, name).unwrap(), first);

    let second = Secret::generate();
    store.set(name, &second).unwrap();
    assert_eq!(store.get(name).unwrap(), Some(second));

    store.delete(name).unwrap();
    assert_eq!(store.get(name).unwrap(), None);
}
