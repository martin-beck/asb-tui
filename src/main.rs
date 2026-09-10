// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use std::{env, process::ExitCode};

const DIAGNOSTIC: &str = concat!(
    "{\"classification\":\"unverified_extension\",",
    "\"protocol\":\"asb-cli-capabilities\",",
    "\"protocol_version\":1,",
    "\"reason\":\"installed_asb_compatibility_not_verified\"}"
);

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments == ["doctor", "--format", "json"] {
        println!("{DIAGNOSTIC}");
        return ExitCode::from(3);
    }
    eprintln!("usage: asb-tui doctor --format json");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::DIAGNOSTIC;

    #[test]
    fn diagnostic_is_content_free_and_explicitly_unverified() {
        assert!(DIAGNOSTIC.contains("unverified_extension"));
        assert!(DIAGNOSTIC.contains("installed_asb_compatibility_not_verified"));
        assert!(!DIAGNOSTIC.contains('/'));
        assert!(!DIAGNOSTIC.contains("token"));
    }
}
