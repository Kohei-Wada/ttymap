//! `ttymap.sgp4` — TLE propagation for satellite plugins.
//!
//! Surface:
//!
//! ```text
//! ttymap.sgp4 :parse_tle(text)            -> handle | nil
//! ttymap.sgp4 :parse_tles(text)           -> array of handles
//! ttymap.sgp4 :propagate(handle [, t])    -> { lon, lat, alt_km, vel_kms } | nil
//! ttymap.sgp4 :propagate_batch(handles [, t]) -> array (false on per-item fail)
//! ```
//!
//! `t` is a unix timestamp in seconds (fractional accepted). Omitted
//! → current wall-clock. The propagator runs in microseconds so a
//! handful of satellites can be re-propagated every poll cycle for
//! smooth motion; `propagate_batch` exists so group plugins (Starlink
//! et al.) keep the Lua/Rust crossing to one call regardless of
//! satellite count.
//!
//! Output coordinates are WGS-84 geodetic (lon/lat in degrees,
//! altitude in km above ellipsoid). The TEME → geodetic step uses
//! `sgp4::iau_epoch_to_sidereal_time` for GMST and a closed-form
//! Bowring conversion — accurate to a few meters at LEO, well under
//! a Braille pixel at any reasonable zoom.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mlua::{AnyUserData, Lua, Table, UserData};
use sgp4::{Constants, Elements, MinutesSinceEpoch};

/// Unix timestamp of the J2000 epoch (UTC 2000-01-01 12:00:00).
const J2000_UNIX_SECS: f64 = 946_728_000.0;
/// Seconds in one Julian year — matches `sgp4::Elements::epoch`'s
/// "years since J2000" definition.
const SECS_PER_JULIAN_YEAR: f64 = 365.25 * 86400.0;

// ── ttymap.sgp4 namespace ───────────────────────────────────────────

pub struct HostSgp4;

impl UserData for HostSgp4 {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.sgp4:parse_tle(text)` — accepts either 3-line
        // format (object name + 2 element lines, CelesTrak default)
        // or bare 2-line format. Returns nil on parse failure (a
        // logged warning, no panic) so a flaky upstream doesn't
        // crash a plugin.
        methods.add_method("parse_tle", |lua, _this, text: String| {
            match parse_one(&text) {
                Some(t) => Ok(Some(lua.create_userdata(t)?)),
                None => {
                    log::warn!("lua-host: sgp4:parse_tle failed");
                    Ok(None)
                }
            }
        });

        // `ttymap.sgp4:parse_tles(text)` — multi-TLE block as
        // returned by CelesTrak group endpoints (`FORMAT=tle`,
        // 3-line). Falls back to `FORMAT=2le` when the input has no
        // name lines. Returns a 1-indexed array of handles; an
        // empty array means parse failed entirely.
        methods.add_method("parse_tles", |lua, _this, text: String| {
            let table = lua.create_table()?;
            for (i, tle) in parse_many(&text).into_iter().enumerate() {
                table.set(i + 1, lua.create_userdata(tle)?)?;
            }
            Ok(table)
        });

        // `ttymap.sgp4:propagate(handle [, unix_time])` — propagate
        // to wall-clock now (or `unix_time` if provided) and return
        // `{lon, lat, alt_km, vel_kms}`. Nil on propagate error so
        // a degraded TLE just stops moving rather than crashing.
        methods.add_method(
            "propagate",
            |lua, _this, (handle, t): (AnyUserData, Option<f64>)| {
                let tle = handle.borrow::<LuaTle>()?;
                let when = t.unwrap_or_else(unix_now);
                propagate_one(&tle, when)
                    .map(|p| position_to_lua(lua, &p))
                    .transpose()
            },
        );

        // `ttymap.sgp4:propagate_batch(handles [, unix_time])` — same
        // as propagate but over a 1-indexed array of handles. The
        // returned array has the same length; failed entries are
        // `false` (not nil — keeps the array contiguous so `ipairs`
        // and `#t` work). Propagation is microsecond-cheap; for
        // thousands of sats the overhead is dominated by the table
        // round-trip, which is why this lives on the Rust side.
        methods.add_method(
            "propagate_batch",
            |lua, _this, (handles, t): (Table, Option<f64>)| {
                let when = t.unwrap_or_else(unix_now);
                let result = lua.create_table()?;
                let len = handles.raw_len();
                for i in 1..=len {
                    let h: AnyUserData = handles.raw_get(i)?;
                    let tle = h.borrow::<LuaTle>()?;
                    let row = match propagate_one(&tle, when) {
                        Some(p) => mlua::Value::Table(position_to_lua(lua, &p)?),
                        None => mlua::Value::Boolean(false),
                    };
                    result.set(i, row)?;
                }
                Ok(result)
            },
        );
    }
}

// ── LuaTle handle ──────────────────────────────────────────────────

/// Parsed TLE handle exposed to Lua. `Constants` is non-trivial
/// (~hundred f64s) and not `Clone`, but is immutable post-construction
/// — `Arc` lets `parse_tles` hand out cheap clones if a plugin ever
/// wants to keep separate copies, while still satisfying mlua's
/// `'static` userdata bound.
pub struct LuaTle {
    inner: Arc<TleInner>,
}

struct TleInner {
    name: Option<String>,
    /// Unix seconds at the TLE epoch. Pre-computed once so per-call
    /// propagation is just a subtraction + divide.
    epoch_unix: f64,
    constants: Constants,
}

impl UserData for LuaTle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `tle:name() -> string | nil` — convenience for plugins
        // that want the satellite's name without keeping it
        // alongside the handle.
        methods.add_method("name", |_, this, _: ()| Ok(this.inner.name.clone()));
    }
}

// ── parse helpers ──────────────────────────────────────────────────

fn parse_one(text: &str) -> Option<LuaTle> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let elements = match lines.as_slice() {
        [name, l1, l2] => Elements::from_tle(
            Some((*name).trim().to_owned()),
            l1.as_bytes(),
            l2.as_bytes(),
        )
        .ok()?,
        [l1, l2] => Elements::from_tle(None, l1.as_bytes(), l2.as_bytes()).ok()?,
        _ => return None,
    };
    inner_from(elements).map(LuaTle::wrap)
}

fn parse_many(text: &str) -> Vec<LuaTle> {
    // Trim blank lines so a stray trailing newline doesn't desync the
    // parser's 3-line cadence.
    let cleaned: String = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    // First non-blank line starts with "1 " → bare 2LE format. Else
    // assume 3LE (name first). CelesTrak's FORMAT=tle is 3LE.
    let is_2le = cleaned.lines().next().is_some_and(|l| l.starts_with("1 "));

    let elements_vec = if is_2le {
        sgp4::parse_2les(&cleaned)
    } else {
        sgp4::parse_3les(&cleaned)
    };

    match elements_vec {
        Ok(v) => v
            .into_iter()
            .filter_map(inner_from)
            .map(LuaTle::wrap)
            .collect(),
        Err(e) => {
            log::warn!("lua-host: sgp4:parse_tles failed: {e}");
            Vec::new()
        }
    }
}

fn inner_from(elements: Elements) -> Option<TleInner> {
    let constants = Constants::from_elements(&elements).ok()?;
    let epoch_unix = elements.epoch() * SECS_PER_JULIAN_YEAR + J2000_UNIX_SECS;
    let name = elements.object_name.clone();
    Some(TleInner {
        name,
        epoch_unix,
        constants,
    })
}

impl LuaTle {
    fn wrap(inner: TleInner) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }
}

// ── propagate ──────────────────────────────────────────────────────

struct Position {
    lon_deg: f64,
    lat_deg: f64,
    alt_km: f64,
    vel_kms: f64,
}

fn propagate_one(tle: &LuaTle, unix_time: f64) -> Option<Position> {
    let mse = MinutesSinceEpoch((unix_time - tle.inner.epoch_unix) / 60.0);
    let pred = match tle.inner.constants.propagate(mse) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("lua-host: sgp4:propagate failed: {e}");
            return None;
        }
    };
    let years_since_j2000 = (unix_time - J2000_UNIX_SECS) / SECS_PER_JULIAN_YEAR;
    let gmst = sgp4::iau_epoch_to_sidereal_time(years_since_j2000);
    Some(teme_to_geodetic(&pred, gmst))
}

/// Rotate TEME → ECEF by GMST around Z, then ECEF → WGS-84 geodetic
/// via the Bowring closed form. Accurate to better than 10 m at LEO
/// — well below a Braille pixel at any zoom we care about.
fn teme_to_geodetic(pred: &sgp4::Prediction, gmst: f64) -> Position {
    const A_KM: f64 = 6378.137;
    const F: f64 = 1.0 / 298.257_223_563;
    const B_KM: f64 = A_KM * (1.0 - F);
    const E2: f64 = 1.0 - (B_KM * B_KM) / (A_KM * A_KM);
    const EP2: f64 = (A_KM * A_KM) / (B_KM * B_KM) - 1.0;

    let [xt, yt, zt] = pred.position;
    let cos_g = gmst.cos();
    let sin_g = gmst.sin();
    let xe = cos_g * xt + sin_g * yt;
    let ye = -sin_g * xt + cos_g * yt;
    let ze = zt;

    let p = (xe * xe + ye * ye).sqrt();
    let lon = ye.atan2(xe);
    let theta = (ze * A_KM).atan2(p * B_KM);
    let lat = (ze + EP2 * B_KM * theta.sin().powi(3)).atan2(p - E2 * A_KM * theta.cos().powi(3));
    let n = A_KM / (1.0 - E2 * lat.sin().powi(2)).sqrt();
    let alt = p / lat.cos() - n;

    let [vx, vy, vz] = pred.velocity;
    let vel_kms = (vx * vx + vy * vy + vz * vz).sqrt();

    Position {
        lon_deg: lon.to_degrees(),
        lat_deg: lat.to_degrees(),
        alt_km: alt,
        vel_kms,
    }
}

fn position_to_lua(lua: &Lua, p: &Position) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("lon", p.lon_deg)?;
    t.set("lat", p.lat_deg)?;
    t.set("alt_km", p.alt_km)?;
    t.set("vel_kms", p.vel_kms)?;
    Ok(t)
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-good ISS TLE (lifted from sgp4 crate's own tests, so the
    /// checksums verify). We don't assert exact lat/lon (those drift
    /// with epoch); we just check the outputs are in valid ranges
    /// and the altitude is plausibly LEO.
    const ISS_3LE: &str = "ISS (ZARYA)
1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927
2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";

    #[test]
    fn parse_tle_accepts_3line_format() {
        let tle = parse_one(ISS_3LE).expect("parse must succeed");
        assert_eq!(tle.inner.name.as_deref(), Some("ISS (ZARYA)"));
    }

    #[test]
    fn parse_tle_accepts_2line_format() {
        let two_line: String = ISS_3LE.lines().skip(1).collect::<Vec<_>>().join("\n");
        let tle = parse_one(&two_line).expect("parse must succeed");
        assert!(tle.inner.name.is_none());
    }

    #[test]
    fn parse_tle_rejects_garbage() {
        assert!(parse_one("not a tle").is_none());
        assert!(parse_one("").is_none());
    }

    #[test]
    fn propagate_at_epoch_yields_lowearth_orbit_altitude() {
        let tle = parse_one(ISS_3LE).expect("parse");
        // Propagate at the TLE epoch itself.
        let pos = propagate_one(&tle, tle.inner.epoch_unix).expect("propagate");
        // ISS is ~400 km up. Allow generous slack for SGP4 error +
        // ellipsoid vs spherical altitude conversion.
        assert!(
            (350.0..500.0).contains(&pos.alt_km),
            "altitude {} km out of LEO band",
            pos.alt_km
        );
        // ISS orbital speed is ~7.66 km/s.
        assert!(
            (7.0..8.0).contains(&pos.vel_kms),
            "velocity {} km/s out of expected band",
            pos.vel_kms
        );
        // lat/lon must be in valid ranges.
        assert!((-90.0..=90.0).contains(&pos.lat_deg));
        assert!((-180.0..=180.0).contains(&pos.lon_deg));
    }

    #[test]
    fn propagate_advances_position_over_time() {
        let tle = parse_one(ISS_3LE).expect("parse");
        let p0 = propagate_one(&tle, tle.inner.epoch_unix).expect("p0");
        // 60 s later the satellite has moved ~460 km along its
        // orbit — definitely a different lon/lat.
        let p1 = propagate_one(&tle, tle.inner.epoch_unix + 60.0).expect("p1");
        let dlat = (p0.lat_deg - p1.lat_deg).abs();
        let dlon = (p0.lon_deg - p1.lon_deg).abs();
        assert!(
            dlat + dlon > 0.5,
            "expected motion, got dlat={} dlon={}",
            dlat,
            dlon
        );
    }

    #[test]
    fn parse_tles_handles_multi_block_3le() {
        // Two ISS-like blocks back to back. Same TLE twice is fine
        // for the parser's purposes.
        let multi = format!("{}\n{}", ISS_3LE, ISS_3LE);
        let tles = parse_many(&multi);
        assert_eq!(tles.len(), 2);
    }
}
