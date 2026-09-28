//! Serial date arithmetic (1900 system with the 1900-02-29 bug, and 1904).

use super::value::{ErrorKind, R};

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's algorithm).
pub(crate) const fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub(crate) const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

const D18991230: i64 = days_from_civil(1899, 12, 30);
const D18991231: i64 = days_from_civil(1899, 12, 31);
const D19000301: i64 = days_from_civil(1900, 3, 1);
const D19040101: i64 = days_from_civil(1904, 1, 1);
/// Largest valid serial (9999-12-31) in the 1900 system.
pub(crate) const MAX_SERIAL_1900: i64 = 2_958_465;
pub(crate) const MAX_SERIAL_1904: i64 = MAX_SERIAL_1900 - 1462;

pub(crate) fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub(crate) fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) || y == 1900 {
                29
            } else {
                28
            }
        }
    }
}

/// Serial of a concrete (valid) calendar date.
pub(crate) fn serial_from_ymd(y: i64, m: u32, d: u32, d1904: bool) -> i64 {
    if !d1904 && y == 1900 && m == 2 && d == 29 {
        return 60;
    }
    let days = days_from_civil(y, m, d);
    if d1904 {
        days - D19040101
    } else if days < D19000301 {
        days - D18991231
    } else {
        days - D18991230
    }
}

pub(crate) fn max_serial(d1904: bool) -> i64 {
    if d1904 { MAX_SERIAL_1904 } else { MAX_SERIAL_1900 }
}

/// `DATE(y, m, d)` semantics: years 0..1899 are offset by 1900, month and
/// day overflow roll over.
pub(crate) fn date_serial(y: f64, m: f64, d: f64, d1904: bool) -> R<f64> {
    if !(y.is_finite() && m.is_finite() && d.is_finite()) {
        return Err(ErrorKind::Num);
    }
    let mut y = y.trunc() as i64;
    if !(0..10000).contains(&y) {
        return Err(ErrorKind::Num);
    }
    if y < 1900 {
        y += 1900;
    }
    let m = m.trunc() as i64;
    let d = d.trunc() as i64;
    let y2 = y + (m - 1).div_euclid(12);
    let m2 = ((m - 1).rem_euclid(12) + 1) as u32;
    if !(0..10000).contains(&y2) {
        return Err(ErrorKind::Num);
    }
    let s = serial_from_ymd(y2, m2, 1, d1904) + d - 1;
    if s < 0 || s > max_serial(d1904) {
        return Err(ErrorKind::Num);
    }
    Ok(s as f64)
}

/// Serial → (year, month, day). Serial 0 is 1900-01-00, 60 is 1900-02-29.
pub(crate) fn serial_to_ymd(serial: f64, d1904: bool) -> R<(i64, u32, u32)> {
    if !serial.is_finite() {
        return Err(ErrorKind::Num);
    }
    let n = serial.floor() as i64;
    if n < 0 || n > max_serial(d1904) {
        return Err(ErrorKind::Num);
    }
    if d1904 {
        return Ok(civil_from_days(n + D19040101));
    }
    Ok(match n {
        0 => (1900, 1, 0),
        60 => (1900, 2, 29),
        n if n < 60 => civil_from_days(n + D18991231),
        n => civil_from_days(n + D18991230),
    })
}

/// Day of week, 0 = Sunday .. 6 = Saturday (Excel's view, including the
/// fictitious 1900-02-29).
pub(crate) fn weekday_sun0(serial: f64, d1904: bool) -> i64 {
    let mut n = serial.floor() as i64;
    if d1904 {
        n += 1462;
    }
    (n - 1).rem_euclid(7)
}

/// Monday = 0 .. Sunday = 6.
pub(crate) fn weekday_mon0(serial: f64, d1904: bool) -> usize {
    ((weekday_sun0(serial, d1904) + 6) % 7) as usize
}

/// Seconds of the day of a serial, rounded to the nearest second.
pub(crate) fn time_parts(serial: f64) -> R<(u32, u32, u32)> {
    if serial < 0.0 || !serial.is_finite() {
        return Err(ErrorKind::Num);
    }
    let frac = serial - serial.floor();
    let secs = ((frac * 86400.0).round() as i64).rem_euclid(86400);
    Ok(((secs / 3600) as u32, ((secs / 60) % 60) as u32, (secs % 60) as u32))
}

const MONTHS: [&str; 12] =
    ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

pub(crate) fn month_name(m: u32) -> &'static str {
    const N: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    N[(m.clamp(1, 12) - 1) as usize]
}

pub(crate) fn day_name(sun0: usize) -> &'static str {
    const N: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    N[sun0 % 7]
}

fn month_from_name(s: &str) -> Option<u32> {
    if s.len() < 3 {
        return None;
    }
    let l = s.to_ascii_lowercase();
    let l = l.trim_end_matches('.');
    MONTHS.iter().position(|m| m.starts_with(l) && (l.len() == 3 || l.len() == m.len() || (l == "sept"))).map(|p| p as u32 + 1)
}

fn year2(y: i64, digits: usize) -> i64 {
    if digits <= 2 { if y < 30 { 2000 + y } else { 1900 + y } } else { y }
}

fn valid(y: i64, m: u32, d: u32) -> bool {
    (1900..=9999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

/// Parses the date part of a string. Accepts ISO `yyyy-mm-dd`,
/// `yyyy/mm/dd`, US `m/d/yyyy`, `d-mmm-yyyy`, `d mmm yyyy`, `mmm d, yyyy`,
/// `mmmm yyyy`.
fn parse_date_part(s: &str, d1904: bool) -> Option<f64> {
    let parts: Vec<&str> = s.split(['-', '/', ' ', ',']).filter(|p| !p.is_empty()).collect();
    let isnum = |p: &str| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit());
    let (y, m, d);
    match parts.len() {
        3 if parts.iter().all(|p| isnum(p)) => {
            let a: Vec<i64> = parts.iter().map(|p| p.parse().unwrap_or(-1)).collect();
            if parts[0].len() >= 3 {
                y = a[0];
                m = a[1] as u32;
                d = a[2] as u32;
            } else {
                m = a[0] as u32;
                d = a[1] as u32;
                y = year2(a[2], parts[2].len());
            }
        }
        3 => {
            if let (true, Some(mm), true) = (isnum(parts[0]), month_from_name(parts[1]), isnum(parts[2])) {
                d = parts[0].parse().ok()?;
                m = mm;
                y = year2(parts[2].parse().ok()?, parts[2].len());
            } else if let (Some(mm), true, true) = (month_from_name(parts[0]), isnum(parts[1]), isnum(parts[2])) {
                m = mm;
                d = parts[1].parse().ok()?;
                y = year2(parts[2].parse().ok()?, parts[2].len());
            } else {
                return None;
            }
        }
        2 => {
            if let (Some(mm), true) = (month_from_name(parts[0]), isnum(parts[1])) {
                if parts[1].len() < 3 {
                    return None;
                }
                m = mm;
                d = 1;
                y = parts[1].parse().ok()?;
            } else {
                return None;
            }
        }
        _ => return None,
    }
    if !valid(y, m, d) {
        return None;
    }
    let s = serial_from_ymd(y, m, d, d1904);
    if s < 0 {
        return None;
    }
    Some(s as f64)
}

/// Parses `h:mm`, `h:mm:ss(.fff)`, with optional `AM`/`PM`; returns days.
pub(crate) fn parse_time(s: &str) -> Option<f64> {
    let t = s.trim().to_ascii_lowercase();
    let (body, ampm) = if let Some(b) = t.strip_suffix("am") {
        (b.trim_end(), Some(false))
    } else if let Some(b) = t.strip_suffix("pm") {
        (b.trim_end(), Some(true))
    } else {
        (t.as_str(), None)
    };
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() > 3 || (parts.len() == 1 && ampm.is_none()) {
        return None;
    }
    let mut h: f64 = parts[0].trim().parse::<u32>().ok()? as f64;
    let m: f64 = if parts.len() > 1 { parts[1].trim().parse::<u32>().ok()? as f64 } else { 0.0 };
    let sec: f64 = if parts.len() > 2 {
        let p = parts[2].trim();
        if !p.bytes().all(|c| c.is_ascii_digit() || c == b'.') || p.is_empty() {
            return None;
        }
        p.parse().ok()?
    } else {
        0.0
    };
    if m >= 60.0 || sec >= 60.0 {
        return None;
    }
    if let Some(pm) = ampm {
        if !(1.0..=12.0).contains(&h) && h != 0.0 {
            return None;
        }
        if h == 12.0 {
            h = 0.0;
        }
        if pm {
            h += 12.0;
        }
    } else if h > 9999.0 {
        return None;
    }
    Some((h * 3600.0 + m * 60.0 + sec) / 86400.0)
}

/// Parses a date, a time, or a date followed by a time.
pub(crate) fn parse_datetime(s: &str, d1904: bool) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains(':') {
        if let Some(t) = parse_time(s) {
            return Some(t);
        }
        // split date and time
        let (dp, tp) = if let Some(p) = s.find('T').filter(|&p| p >= 8 && s[..p].contains('-')) {
            (&s[..p], &s[p + 1..])
        } else {
            let colon = s.find(':')?;
            let sp = s[..colon].rfind(' ')?;
            (&s[..sp], &s[sp + 1..])
        };
        let d = parse_date_part(dp.trim(), d1904)?;
        let t = parse_time(tp)?;
        return Some(d + t);
    }
    if let Some(d) = parse_date_part(s, d1904) {
        return Some(d);
    }
    // "10 am"
    let l = s.to_ascii_lowercase();
    if l.ends_with("am") || l.ends_with("pm") {
        return parse_time(s);
    }
    None
}
