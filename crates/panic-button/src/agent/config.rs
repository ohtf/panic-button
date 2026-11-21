// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The machine config file (`spec/08-deployment.md`, Settings reference).
//!
//! Loading is strict: unknown keys, wrong types and out-of-range values are
//! errors, never silently defaulted (house-style rust-cli.md). A typo in a
//! policy file must not quietly leave a device on defaults.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};

use crate::core::config::{LocalConfig, MachineSettings, Profile, UserPrefs, parse_ipv4_peer};
use crate::proto::{CueMode, SoundPattern};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProfileName {
    #[default]
    Practice,
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Enrolment {
    pub token: Option<String>,
    pub token_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserDisplaySource {
    #[default]
    OsFullName,
    OsUsername,
    RoomOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub room: Option<String>,
    pub label: Option<String>,
    pub user_display_source: Option<UserDisplaySource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub port: Option<u16>,
    pub multicast_group: Option<Ipv4Addr>,
    pub multicast_ttl: Option<u8>,
    #[serde(default)]
    pub seed_peers: Vec<String>,
    #[serde(default)]
    pub relay_urls: Vec<String>,
    pub relay_ca_pem: Option<PathBuf>,
    /// Advertise and browse with mDNS (not in the settings reference; on by
    /// default, for environments where mDNS is unwanted).
    pub mdns: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueModeName {
    Silent,
    Soft,
    AlarmRamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PatternName {
    A,
    B,
    C,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Alarm {
    pub hold_ms: Option<u32>,
    pub cue_mode: Option<CueModeName>,
    pub sound_pattern: Option<PatternName>,
    pub volume_pct: Option<u8>,
    pub override_mute: Option<bool>,
    pub hotkey: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FloatingButton {
    pub allow_hide: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Ui {
    #[serde(default)]
    pub floating_button: FloatingButton,
    pub allow_quit: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Updates {
    Managed,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub allow_remote_session: Option<bool>,
    pub updates: Option<Updates>,
    pub allow_cli_raise: Option<bool>,
}

/// `config.toml` (machine config, source 2) or `policy.toml` (source 1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub profile: Option<ProfileName>,
    #[serde(default)]
    pub enrolment: Enrolment,
    #[serde(default)]
    pub device: Device,
    #[serde(default)]
    pub network: Network,
    #[serde(default)]
    pub alarm: Alarm,
    #[serde(default)]
    pub ui: Ui,
    #[serde(default)]
    pub policy: Policy,
}

impl ConfigFile {
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let c: ConfigFile = toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })?;
        c.validate()?;
        Ok(c)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(&text, path)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(h) = self.alarm.hold_ms
            && !(1000..=3000).contains(&h)
        {
            return Err(ConfigError::Invalid(format!(
                "alarm.hold_ms must be 1000-3000, got {h}"
            )));
        }
        if let Some(v) = self.alarm.volume_pct
            && v > 100
        {
            return Err(ConfigError::Invalid(format!(
                "alarm.volume_pct must be 0-100, got {v}"
            )));
        }
        if let Some(t) = self.network.multicast_ttl
            && !(1..=8).contains(&t)
        {
            return Err(ConfigError::Invalid(format!(
                "network.multicast_ttl must be 1-8, got {t}"
            )));
        }
        if self.enrolment.token.is_some() && self.enrolment.token_file.is_some() {
            return Err(ConfigError::Invalid(
                "set enrolment.token or enrolment.token_file, not both".into(),
            ));
        }
        Ok(())
    }

    /// Merges `policy` (source 1) over `self` (source 2): policy wins per
    /// key; `seed_peers` and `relay_urls` are unions.
    pub fn with_policy(mut self, policy: &ConfigFile) -> ConfigFile {
        macro_rules! over {
            ($($f:ident).+) => {
                if policy.$($f).+.is_some() {
                    self.$($f).+ = policy.$($f).+.clone();
                }
            };
        }
        over!(profile);
        over!(enrolment.token);
        over!(enrolment.token_file);
        over!(device.room);
        over!(device.label);
        over!(device.user_display_source);
        over!(network.port);
        over!(network.multicast_group);
        over!(network.multicast_ttl);
        over!(network.relay_ca_pem);
        over!(network.mdns);
        over!(alarm.hold_ms);
        over!(alarm.cue_mode);
        over!(alarm.sound_pattern);
        over!(alarm.volume_pct);
        over!(alarm.override_mute);
        over!(alarm.hotkey);
        over!(ui.floating_button.allow_hide);
        over!(ui.allow_quit);
        over!(policy.allow_remote_session);
        over!(policy.updates);
        over!(policy.allow_cli_raise);
        for p in &policy.network.seed_peers {
            if !self.network.seed_peers.contains(p) {
                self.network.seed_peers.push(p.clone());
            }
        }
        for u in &policy.network.relay_urls {
            if !self.network.relay_urls.contains(u) {
                self.network.relay_urls.push(u.clone());
            }
        }
        self
    }

    /// Whether Settings offers Quit (`ui.allow_quit`): yes, unless IT sets
    /// it to false (`spec/08-deployment.md`).
    pub fn allow_quit(&self) -> bool {
        self.ui.allow_quit != Some(false)
    }

    /// Whether the tray menu offers Quit too: in the home profile, or when
    /// IT sets `ui.allow_quit = true`.
    pub fn quit_in_tray(&self) -> bool {
        self.ui
            .allow_quit
            .unwrap_or(self.profile == Some(ProfileName::Home))
    }

    pub fn port(&self) -> u16 {
        self.network
            .port
            .unwrap_or(crate::proto::limits::DEFAULT_PORT)
    }

    /// The core's view of sources 1, 2 and 4.
    pub fn local_config(&self, default_label: &str) -> LocalConfig {
        let port = self.port();
        let seed_addrs: Vec<SocketAddrV4> = self
            .network
            .seed_peers
            .iter()
            .filter_map(|s| {
                let a = parse_ipv4_peer(s, port);
                if a.is_none() {
                    tracing::warn!("seed peer {s:?} is not an IPv4 address; ignored");
                }
                a
            })
            .collect();
        LocalConfig {
            profile: match self.profile.unwrap_or_default() {
                ProfileName::Practice => Profile::Practice,
                ProfileName::Home => Profile::Home,
            },
            label: self
                .device
                .label
                .clone()
                .unwrap_or_else(|| default_label.to_owned()),
            room: self.device.room.clone().unwrap_or_default(),
            label_locked: self.device.label.is_some(),
            room_locked: self.device.room.is_some(),
            machine: MachineSettings {
                hold_ms: self.alarm.hold_ms,
                cue_mode: self.alarm.cue_mode.map(|c| match c {
                    CueModeName::Silent => CueMode::Silent,
                    CueModeName::Soft => CueMode::Soft,
                    CueModeName::AlarmRamp => CueMode::AlarmRamp,
                }),
                sound_pattern: self.alarm.sound_pattern.map(|p| match p {
                    PatternName::A => SoundPattern::A,
                    PatternName::B => SoundPattern::B,
                    PatternName::C => SoundPattern::C,
                }),
                override_mute: self.alarm.override_mute,
                volume_pct: self.alarm.volume_pct,
                seed_peers: self.network.seed_peers.clone(),
                relay_urls: self.network.relay_urls.clone(),
            },
            user: UserPrefs::default(),
            port,
            allow_remote_session: self.policy.allow_remote_session.unwrap_or(false),
            seed_addrs,
            relay_count: self.network.relay_urls.len() as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_is_in_settings_unless_it_turns_it_off() {
        let p = |t: &str| ConfigFile::parse(t, Path::new("c.toml")).expect("parse");
        // (in Settings, in the tray)
        let q = |t: &str| (p(t).allow_quit(), p(t).quit_in_tray());
        assert_eq!(q(""), (true, false));
        assert_eq!(q("profile = \"practice\""), (true, false));
        assert_eq!(q("profile = \"home\""), (true, true));
        assert_eq!(q("[ui]\nallow_quit = true"), (true, true));
        assert_eq!(
            q("profile = \"home\"\n[ui]\nallow_quit = false"),
            (false, false)
        );
    }

    fn parse(s: &str) -> Result<ConfigFile, ConfigError> {
        ConfigFile::parse(s, Path::new("test.toml"))
    }

    #[test]
    fn spec_example_parses() {
        // `spec/08-deployment.md`, Example machine config.
        let c = parse(
            r#"
profile = "practice"

[enrolment]
token_file = 'C:\ProgramData\Panic Button\enrol.token'

[device]
room = "Reception"

[network]
seed_peers = ["10.20.1.10:47474"]
relay_urls = ["wss://relay.example-pcn.org.uk/v1/ws"]
"#,
        )
        .expect("parses");
        let l = c.local_config("HOST");
        assert_eq!(l.room, "Reception");
        assert_eq!(l.label, "HOST");
        assert_eq!(l.seed_addrs.len(), 1);
        assert_eq!(l.relay_count, 1);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(parse("[alarm]\nhold_msec = 1500\n").is_err());
        assert!(parse("colour = \"red\"\n").is_err());
        assert!(parse("[alarm]\ncue_mode = \"loud\"\n").is_err());
    }

    #[test]
    fn ranges_are_checked() {
        assert!(parse("[alarm]\nhold_ms = 500\n").is_err());
        assert!(parse("[alarm]\nvolume_pct = 101\n").is_err());
        assert!(parse("[alarm]\nhold_ms = 2000\n").is_ok());
    }

    #[test]
    fn policy_wins_and_lists_merge() {
        let machine =
            parse("[alarm]\nhold_ms = 2000\n[network]\nrelay_urls = [\"a\"]\n").expect("m");
        let policy =
            parse("[alarm]\nhold_ms = 2500\n[network]\nrelay_urls = [\"b\"]\n").expect("p");
        let c = machine.with_policy(&policy);
        assert_eq!(c.alarm.hold_ms, Some(2500));
        assert_eq!(c.network.relay_urls, vec!["a".to_string(), "b".to_string()]);
    }
}
