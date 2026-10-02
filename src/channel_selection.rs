// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral release-channel selection for the standalone lifecycle.

use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseChannel {
    Dev,
    Stable,
    Nightly,
    Experimental,
}

impl ReleaseChannel {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dev" => Some(Self::Dev),
            "stable" => Some(Self::Stable),
            "nightly" => Some(Self::Nightly),
            "experimental" => Some(Self::Experimental),
            _ => None,
        }
    }
}

impl ReleaseChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Stable => "stable",
            Self::Nightly => "nightly",
            Self::Experimental => "experimental",
        }
    }

    pub const fn available(self) -> bool {
        matches!(self, Self::Dev)
    }

    pub const fn warning(self) -> Option<&'static str> {
        match self {
            Self::Dev => None,
            Self::Stable => Some("stable_channel_unavailable"),
            Self::Nightly => Some("nightly_channel_unavailable"),
            Self::Experimental => Some("experimental_channel_unavailable"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelSelection {
    pub requested: ReleaseChannel,
    pub active: ReleaseChannel,
    pub warning: Option<&'static str>,
}

impl ChannelSelection {
    /// A fresh development setup is explicitly defaulted to the dev channel.
    pub const fn fresh() -> Self {
        Self {
            requested: ReleaseChannel::Dev,
            active: ReleaseChannel::Dev,
            warning: None,
        }
    }

    /// Existing state is retained unless the user explicitly requests another
    /// channel. Unavailable channels remain visible as warnings and never
    /// become an implicit production fallback.
    pub const fn for_request(active: ReleaseChannel, requested: Option<ReleaseChannel>) -> Self {
        let requested = match requested {
            Some(value) => value,
            None => active,
        };
        Self {
            requested,
            active: if requested.available() {
                requested
            } else {
                active
            },
            warning: requested.warning(),
        }
    }

    /// Read the last explicit choice. A missing or malformed file is treated as
    /// a fresh setup, while an unavailable channel remains selected and visible
    /// to the caller (it is never silently replaced with dev).
    pub fn persisted() -> Option<ReleaseChannel> {
        let path = state_path()?;
        let bytes = fs::read(path).ok()?;
        let state: PersistedChannel = serde_json::from_slice(&bytes).ok()?;
        if state.schema_version != 1 {
            return None;
        }
        ReleaseChannel::parse(&state.channel)
    }

    /// Persist only the public channel name. This file contains no credentials,
    /// signatures, or trust material and is replaced atomically.
    pub fn persist(channel: ReleaseChannel) -> Result<(), &'static str> {
        let path = state_path().ok_or("channel_state_path_invalid")?;
        let parent = path.parent().ok_or("channel_state_path_invalid")?;
        fs::create_dir_all(parent).map_err(|_| "channel_state_unavailable")?;
        let state = PersistedChannel {
            schema_version: 1,
            channel: channel.as_str().to_owned(),
        };
        let bytes = serde_json::to_vec(&state).map_err(|_| "channel_state_unavailable")?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, bytes).map_err(|_| "channel_state_unavailable")?;
        fs::rename(temporary, path).map_err(|_| "channel_state_unavailable")
    }
}

#[derive(Deserialize, Serialize)]
struct PersistedChannel {
    schema_version: u64,
    channel: String,
}

fn state_path() -> Option<PathBuf> {
    if let Some(value) = env::var_os("ASB_TUI_CHANNEL_STATE") {
        let path = PathBuf::from(value);
        return path.is_absolute().then_some(path);
    }
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
    Some(base.join("asb-tui").join("channel.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_selection_defaults_to_development() {
        assert_eq!(
            ChannelSelection::fresh(),
            ChannelSelection::for_request(ReleaseChannel::Dev, None)
        );
    }

    #[test]
    fn unavailable_channels_warn_without_replacing_active_channel() {
        let selected =
            ChannelSelection::for_request(ReleaseChannel::Dev, Some(ReleaseChannel::Stable));
        assert_eq!(selected.active, ReleaseChannel::Dev);
        assert_eq!(selected.warning, Some("stable_channel_unavailable"));
    }

    #[test]
    fn explicit_dev_selection_has_no_warning() {
        let selected =
            ChannelSelection::for_request(ReleaseChannel::Stable, Some(ReleaseChannel::Dev));
        assert_eq!(selected.active, ReleaseChannel::Dev);
        assert_eq!(selected.warning, None);
    }

    #[test]
    fn channel_names_and_availability_are_closed_and_renderer_neutral() {
        for (name, channel, available, warning) in [
            ("dev", ReleaseChannel::Dev, true, None),
            (
                "stable",
                ReleaseChannel::Stable,
                false,
                Some("stable_channel_unavailable"),
            ),
            (
                "nightly",
                ReleaseChannel::Nightly,
                false,
                Some("nightly_channel_unavailable"),
            ),
            (
                "experimental",
                ReleaseChannel::Experimental,
                false,
                Some("experimental_channel_unavailable"),
            ),
        ] {
            assert_eq!(ReleaseChannel::parse(name), Some(channel));
            assert_eq!(channel.as_str(), name);
            assert_eq!(channel.available(), available);
            assert_eq!(channel.warning(), warning);
        }
        assert_eq!(ReleaseChannel::parse("unknown"), None);
    }

    #[test]
    fn absent_request_retains_active_channel_and_warning() {
        let selected = ChannelSelection::for_request(ReleaseChannel::Nightly, None);
        assert_eq!(selected.requested, ReleaseChannel::Nightly);
        assert_eq!(selected.active, ReleaseChannel::Nightly);
        assert_eq!(selected.warning, Some("nightly_channel_unavailable"));
    }
}
