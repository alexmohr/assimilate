// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

enum Status {
    Active,
    Inactive,
}

fn bad_if_eq(status: &str) -> bool {
    if status == "active" {
        true
    } else {
        false
    }
}

fn bad_if_ne(status: String) -> bool {
    status != "inactive"
}

fn bad_match(status: &str) -> Status {
    match status {
        "active" => Status::Active,
        "inactive" => Status::Inactive,
        _ => Status::Inactive,
    }
}

fn bad_match_or(status: &str) -> bool {
    match status {
        "active" | "enabled" => true,
        _ => false,
    }
}

fn good_enum_compare(status: Status) -> bool {
    matches!(status, Status::Active)
}

fn good_dynamic_compare(a: &str, b: &str) -> bool {
    a == b
}

fn good_match_on_enum(status: Status) -> bool {
    match status {
        Status::Active => true,
        Status::Inactive => false,
    }
}

impl std::str::FromStr for Status {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Status::Active),
            "inactive" => Ok(Status::Inactive),
            _ => Err(()),
        }
    }
}

impl From<&str> for Status {
    fn from(s: &str) -> Self {
        if s == "active" { Status::Active } else { Status::Inactive }
    }
}

struct FakeDeserializer;

impl FakeDeserializer {
    fn deserialize(s: &str) -> Status {
        match s {
            "active" => Status::Active,
            _ => Status::Inactive,
        }
    }
}

#[test]
fn test_uses_string_comparison() {
    let status = "active";
    assert!(status == "active");
}

fn main() {}
