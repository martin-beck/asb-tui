// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use asb_tui::{GitProbe, verify_lock_signature, verify_release_lock_contents};
use std::{env, fs, path::Path, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 5 {
        eprintln!(
            "usage: verify-release-lock LOCK SIGNATURE ALLOWED_SIGNERS COORDINATOR_REPO QUALITY_REPO"
        );
        return ExitCode::from(2);
    }
    let result = verify_lock_signature(
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
    )
    .and_then(|()| {
        fs::read_to_string(&args[0]).map_err(|error| format!("cannot read release lock: {error}"))
    })
    .and_then(|contents| {
        verify_release_lock_contents(
            &contents,
            &GitProbe {
                coordinator: Path::new(&args[3]),
                quality: Path::new(&args[4]),
                allowed_signers: Path::new(&args[2]),
            },
        )
    });
    match result {
        Ok(()) => {
            println!("authenticated immutable release lock verified");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("release verification failed: {error}");
            ExitCode::from(1)
        }
    }
}
