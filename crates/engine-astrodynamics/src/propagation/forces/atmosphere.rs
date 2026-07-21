//! Earth's neutral atmosphere for drag (spec §4.7): NRLMSISE-00 via the
//! pinned `tobari` crate, driven through its low-level numeric input, with
//! the crate-owned space-weather table parsed from the embedded CelesTrak
//! `SW-All.csv`.
//!
//! OBSERVED-ONLY POLICY (spec §4.7, owner-accepted): F10.7 and Ap are
//! measured historical data, and a wrong value silently corrupts drag - so
//! an epoch outside the table's observed span (before its start or past
//! the last OBS/INT row, e.g. with a stale snapshot or a future scene)
//! FAILS LOUDLY instead of substituting defaults. Delete the cached
//! `SW-All.csv` in `OUT_DIR` and rebuild to refresh the snapshot.

use std::sync::LazyLock;

use glam::DVec3;
use hifitime::Epoch;
use tobari::nrlmsise00::{Nrlmsise00, Nrlmsise00Input};
use tobari::space_weather::ConstantWeather;

use crate::propagation::bodies::{AtmosphereModel, CentralBody, Vacuum};

/// NRLMSISE-00's validity ceiling; above it the drag term is skipped
/// entirely (spec §4.7 - the model is also the expensive part of the
/// derivative, so this is a performance cutoff as much as a validity one).
const MODEL_CEILING_KM: f64 = 1000.0;

/// The parsed `SW-All.csv`: one record per day, plus the 3-hourly Ap
/// series flattened for O(1) history lookups.
struct SpaceWeatherTable {
    /// MJD (UTC) of the first record.
    first_mjd: i64,
    /// Daily observed F10.7 (`F10.7_OBS`), one per day.
    f107_observed: Vec<f64>,
    /// Centered 81-day mean (`F10.7_OBS_CENTER81`), one per day.
    f107_center81: Vec<f64>,
    /// Daily Ap (`AP_AVG`), one per day.
    ap_daily: Vec<f64>,
    /// 3-hourly Ap (`AP1..AP8`), eight per day, day-major.
    ap_3hourly: Vec<f64>,
}

static SPACE_WEATHER: LazyLock<SpaceWeatherTable> =
    LazyLock::new(|| SpaceWeatherTable::parse(crate::data::SPACE_WEATHER_CSV));

/// Gregorian (UTC) calendar date to Modified Julian Day number
/// (Fliegel-Van Flandern).
fn mjd_from_ymd(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    let jdn = day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045;
    jdn - 2_400_001
}

impl SpaceWeatherTable {
    /// Panics on malformed data (a broken build) - the embedded snapshot
    /// either parses or the crate should not ship.
    fn parse(csv: &[u8]) -> Self {
        let text = std::str::from_utf8(csv).expect("SW-All.csv is UTF-8");
        let mut lines = text.lines();
        let header: Vec<&str> = lines
            .next()
            .expect("SW-All.csv header")
            .split(',')
            .collect();
        let column = |name: &str| {
            header
                .iter()
                .position(|&h| h == name)
                .unwrap_or_else(|| panic!("SW-All.csv column {name} missing"))
        };
        let date_col = column("DATE");
        let ap_cols: Vec<usize> = (1..=8).map(|i| column(&format!("AP{i}"))).collect();
        let ap_avg_col = column("AP_AVG");
        let f107_col = column("F10.7_OBS");
        let center81_col = column("F10.7_OBS_CENTER81");
        let data_type_col = column("F10.7_DATA_TYPE");

        let mut table = Self {
            first_mjd: 0,
            f107_observed: Vec::new(),
            f107_center81: Vec::new(),
            ap_daily: Vec::new(),
            ap_3hourly: Vec::new(),
        };
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(',').collect();
            // Observed-only: OBS plus INT (gap-interpolated observations).
            // The first PRD (predicted) row ends the usable table.
            let data_type = fields[data_type_col];
            if data_type != "OBS" && data_type != "INT" {
                break;
            }
            let mut date_parts = fields[date_col].split('-');
            let mut next_number = || -> i64 {
                date_parts
                    .next()
                    .and_then(|part| part.parse().ok())
                    .expect("SW-All.csv date")
            };
            let mjd = mjd_from_ymd(next_number(), next_number(), next_number());
            if table.f107_observed.is_empty() {
                table.first_mjd = mjd;
            } else {
                assert_eq!(
                    mjd,
                    table.first_mjd + table.f107_observed.len() as i64,
                    "SW-All.csv days are not contiguous"
                );
            }
            let number = |at: usize| -> f64 {
                fields[at]
                    .parse()
                    .unwrap_or_else(|_| panic!("bad SW-All.csv number {:?}", fields[at]))
            };
            table.f107_observed.push(number(f107_col));
            table.f107_center81.push(number(center81_col));
            table.ap_daily.push(number(ap_avg_col));
            for &col in &ap_cols {
                table.ap_3hourly.push(number(col));
            }
        }
        assert!(
            table.f107_observed.len() > 100,
            "SW-All.csv parsed suspiciously few observed days"
        );
        table
    }

    /// NRLMSISE-00's space-weather inputs at `epoch`, honoring the model's
    /// conventions: daily F10.7 is the PREVIOUS day's value, the 81-day
    /// mean is centered on the epoch, and the 7-element `ap` array carries
    /// the 3-hourly storm history back 57 hours.
    fn weather_at(&self, epoch: Epoch) -> Result<(f64, f64, f64, [f64; 7]), String> {
        let mjd = epoch.to_mjd_utc_days();
        let day = mjd.floor() as i64;
        let index = day - self.first_mjd;
        let last = self.first_mjd + self.f107_observed.len() as i64 - 1;
        // The Ap history reaches 57 h back, so the first three days are
        // unusable too.
        if index < 3 || day > last {
            return Err(format!(
                "no observed space weather for {epoch}: the embedded SW-All.csv covers \
                 MJD {} to {last} (observed); delete the cached file and rebuild to refresh, \
                 or keep drag-bearing scenes inside the observed span",
                self.first_mjd + 3
            ));
        }
        let index = index as usize;
        let slot = (((mjd - day as f64) * 86_400.0) / 10_800.0)
            .floor()
            .min(7.0) as usize;

        let f107_daily = self.f107_observed[index - 1];
        let f107_avg = self.f107_center81[index];
        let ap_daily = self.ap_daily[index];
        // 3-hourly Ap at `slots_back` three-hour intervals before now.
        let ap_back = |slots_back: usize| self.ap_3hourly[index * 8 + slot - slots_back];
        let window_mean =
            |from: usize, to: usize| (from..=to).map(ap_back).sum::<f64>() / (to - from + 1) as f64;
        let ap_array = [
            ap_daily,
            ap_back(0),
            ap_back(1),
            ap_back(2),
            ap_back(3),
            window_mean(4, 11),  // 12-33 h before
            window_mean(12, 19), // 36-57 h before
        ];
        Ok((f107_daily, f107_avg, ap_daily, ap_array))
    }
}

/// NRLMSISE-00 for Earth. The tobari model is driven through its
/// low-level numeric input; the placeholder provider is never consulted.
pub(crate) struct EarthAtmosphere {
    model: Nrlmsise00<ConstantWeather>,
}

impl EarthAtmosphere {
    pub(crate) fn new() -> Self {
        Self {
            model: Nrlmsise00::new(ConstantWeather::new(0.0, 0.0)),
        }
    }
}

impl AtmosphereModel for EarthAtmosphere {
    fn density_kg_m3(&self, body_fixed_m: DVec3, epoch: Epoch) -> Result<Option<f64>, String> {
        // Geodetic (not geocentric) coordinates, via the crate's own WGS84
        // conversion (spec §4.7).
        let geodetic = crate::geodetic::geodetic_from_itrf(body_fixed_m);
        let altitude_km = geodetic.altitude_m / 1e3;
        if altitude_km > MODEL_CEILING_KM {
            return Ok(None);
        }

        let (f107_daily, f107_avg, ap_daily, ap_array) = SPACE_WEATHER.weather_at(epoch)?;
        let (_, _, _, hour, minute, second, nanos) = epoch.to_gregorian_utc();
        let ut_seconds = f64::from(hour) * 3600.0
            + f64::from(minute) * 60.0
            + f64::from(second)
            + f64::from(nanos) * 1e-9;
        let longitude_deg = geodetic.longitude_rad.to_degrees();
        let local_solar_time_hours = (ut_seconds / 3600.0 + longitude_deg / 15.0).rem_euclid(24.0);

        let input = Nrlmsise00Input {
            day_of_year: epoch.day_of_year().floor() as u32,
            ut_seconds,
            altitude_km,
            latitude_deg: geodetic.latitude_rad.to_degrees(),
            longitude_deg,
            local_solar_time_hours,
            f107_daily,
            f107_avg,
            ap_daily,
            ap_array,
        };
        Ok(Some(self.model.calculate(&input).total_mass_density))
    }
}

/// The atmosphere registry (spec §4.7, same pattern as gravity): vacuum is
/// the universal default; Earth resolves to NRLMSISE-00. A Mars/Venus/
/// Titan model later is a new arm here, zero changes to the drag force.
pub(crate) fn atmosphere_for(central: &CentralBody) -> Box<dyn AtmosphereModel> {
    if central.naif_id == 399 {
        Box::new(EarthAtmosphere::new())
    } else {
        Box::new(Vacuum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mid-2024 LEO density lands in the textbook envelope, decreasing
    /// with altitude, and the ceiling cut returns None.
    #[test]
    fn density_envelope_and_ceiling() {
        let atmosphere = EarthAtmosphere::new();
        let epoch = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let at = |altitude_m: f64| {
            atmosphere
                .density_kg_m3(DVec3::new(6_378_137.0 + altitude_m, 0.0, 0.0), epoch)
                .unwrap()
        };
        let d400 = at(400e3).expect("400 km is inside the model");
        assert!(
            (1e-13..1e-10).contains(&d400),
            "rho(400 km) = {d400:.3e} kg/m^3"
        );
        let d300 = at(300e3).expect("300 km is inside the model");
        assert!(d300 > d400, "density must fall with altitude");
        assert_eq!(at(1500e3), None, "above the ceiling the term is skipped");
    }

    /// The observed-only policy fails loudly outside the snapshot's
    /// observed span - both far future (predictions) and pre-table past.
    #[test]
    fn observed_only_policy_fails_loudly() {
        let atmosphere = EarthAtmosphere::new();
        let position = DVec3::new(6_378_137.0 + 400e3, 0.0, 0.0);
        for epoch in [
            Epoch::from_gregorian_utc(2050, 1, 1, 0, 0, 0, 0),
            Epoch::from_gregorian_utc(1957, 10, 2, 0, 0, 0, 0),
        ] {
            let error = atmosphere
                .density_kg_m3(position, epoch)
                .expect_err("outside the observed span");
            assert!(error.contains("space weather"), "unhelpful error: {error}");
        }
    }

    /// Solar activity moves the thermosphere: solar-max 1990 must be far
    /// denser at 400 km than solar-min 1996 - the table is really wired
    /// into the model.
    #[test]
    fn solar_cycle_visible_in_density() {
        let atmosphere = EarthAtmosphere::new();
        let position = DVec3::new(6_378_137.0 + 400e3, 0.0, 0.0);
        let solar_max = atmosphere
            .density_kg_m3(position, Epoch::from_gregorian_utc(1990, 3, 1, 12, 0, 0, 0))
            .unwrap()
            .unwrap();
        let solar_min = atmosphere
            .density_kg_m3(position, Epoch::from_gregorian_utc(1996, 6, 1, 12, 0, 0, 0))
            .unwrap()
            .unwrap();
        assert!(
            solar_max > 3.0 * solar_min,
            "solar max {solar_max:.3e} vs min {solar_min:.3e}"
        );
    }
}
