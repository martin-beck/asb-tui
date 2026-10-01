// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral release-channel selection for the standalone lifecycle.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseChannel {
    Dev,
    Stable,
    Nightly,
    Experimental,
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
}
