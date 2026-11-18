//! Steam Controller (2026) input reports. No output/feature reports are sent.
//!
//! Layout: TritonLib's TritonBatteryStatus_t and EChargeState; also used by
//! SDL's SDL_hidapi_steam_triton.c.
//! https://github.com/Pixel1011/TritonLib/blob/master/include/TritonController.h

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeState {
    Reset,
    Discharging,
    Charging,
    ValidatingSource,
    Full,
    Unknown(u8),
}

impl From<u8> for ChargeState {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Reset,
            1 => Self::Discharging,
            2 => Self::Charging,
            3 => Self::ValidatingSource,
            4 => Self::Full,
            other => Self::Unknown(other),
        }
    }
}

impl fmt::Display for ChargeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reset => f.write_str("Charge state reset"),
            Self::Discharging => f.write_str("Discharging"),
            Self::Charging => f.write_str("Charging"),
            Self::ValidatingSource => f.write_str("Validating power source"),
            Self::Full => f.write_str("Fully charged"),
            Self::Unknown(code) => write!(f, "Unknown charge state (0x{code:02x})"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charge: ChargeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Battery(Battery),
    Activity,
    Disconnected,
}

pub fn parse(data: &[u8]) -> Option<Report> {
    match data {
        // Report ID + complete 14-byte battery payload; trailing padding is OK.
        [0x43, state, percent, ..] if data.len() >= 15 && *percent <= 100 => {
            Some(Report::Battery(Battery {
                percent: *percent,
                charge: (*state).into(),
            }))
        }
        [0x42 | 0x45 | 0x47, _, ..] => Some(Report::Activity),
        [0x79 | 0x46, 1, ..] => Some(Report::Disconnected),
        [0x79 | 0x46, 2, ..] => Some(Report::Activity),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_reports_preserve_percentage_and_all_charge_states() {
        for (code, charge) in [
            (0, ChargeState::Reset),
            (1, ChargeState::Discharging),
            (2, ChargeState::Charging),
            (3, ChargeState::ValidatingSource),
            (4, ChargeState::Full),
            (255, ChargeState::Unknown(255)),
        ] {
            for percent in [0, 57, 100] {
                let mut packet = [0; 15];
                packet[..3].copy_from_slice(&[0x43, code, percent]);
                assert_eq!(
                    parse(&packet),
                    Some(Report::Battery(Battery { percent, charge }))
                );
            }
        }
    }

    #[test]
    fn malformed_and_unrelated_reports_never_become_battery_readings() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[0x43, 2, 80]), None);
        let mut packet = [0; 15];
        packet[..3].copy_from_slice(&[0x43, 2, 101]);
        assert_eq!(parse(&packet), None);
        packet[0] = 0x7b;
        assert_eq!(parse(&packet), None);
        assert_eq!(parse(&[0x42, 0]), Some(Report::Activity));
        assert_eq!(parse(&[0x79, 1]), Some(Report::Disconnected));
    }
}
