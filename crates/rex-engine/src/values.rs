//! Value parsers shared by transforms and coercion (engine spec §8.4–§8.6).

use std::sync::OnceLock;

use chrono::{
    DateTime, Datelike as _, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, SubsecRound as _, TimeZone,
    Utc,
};
use chrono_tz::Tz;
use regex::Regex;
use rex_proto::rex::extract::v1 as pb;
use url::Url;

// ---------------------------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DatetimeSpec {
    pub formats: Vec<String>,
    pub tz: Option<Tz>,
    pub locale: String,
    pub relative: bool,
}

/// A parsed date/time before zone resolution.
#[derive(Clone, Debug, PartialEq)]
pub enum ParsedDate {
    Zoned(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
    Date(NaiveDate),
}

impl ParsedDate {
    /// Instant in UTC; offset-less values are interpreted in `tz` (UTC when absent).
    pub fn to_utc(&self, tz: Option<Tz>) -> Option<DateTime<Utc>> {
        match self {
            ParsedDate::Zoned(d) => Some(d.with_timezone(&Utc)),
            ParsedDate::Naive(n) => local_to_utc(*n, tz),
            ParsedDate::Date(d) => local_to_utc(d.and_time(NaiveTime::MIN), tz),
        }
    }

    /// Calendar date as written (no zone conversion).
    pub fn local_date(&self) -> NaiveDate {
        match self {
            ParsedDate::Zoned(d) => d.date_naive(),
            ParsedDate::Naive(n) => n.date(),
            ParsedDate::Date(d) => *d,
        }
    }
}

fn local_to_utc(n: NaiveDateTime, tz: Option<Tz>) -> Option<DateTime<Utc>> {
    match tz {
        None => Some(Utc.from_utc_datetime(&n)),
        // A wall time inside a DST gap ("02:30" on a spring-forward day) does not exist; read it as the
        // moment the clocks jump to, as browsers do.
        Some(z) => z
            .from_local_datetime(&n)
            .earliest()
            .or_else(|| z.from_local_datetime(&(n + Duration::hours(1))).earliest())
            .map(|d| d.with_timezone(&Utc)),
    }
}

const ZONED_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f%z",
    "%Y-%m-%dT%H:%M:%S%z",
    "%Y-%m-%dT%H:%M%z",
    "%Y-%m-%d %H:%M:%S%.f%z",
    "%Y-%m-%d %H:%M:%S%z",
    "%Y-%m-%d %H:%M:%S %z",
];
const NAIVE_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d %H:%M",
    "%Y/%m/%d %H:%M:%S",
    "%Y/%m/%d %H:%M",
];
const DATE_FORMATS: &[&str] = &["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"];

fn zh_date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(\d{4})\s*年\s*(\d{1,2})\s*月\s*(\d{1,2})\s*日(?:\s*(\d{1,2})\s*[:：时]\s*(\d{1,2})(?:\s*[:：分]\s*(\d{1,2}))?)?")
            .expect("static regex")
    })
}

fn tz_abbrev_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+[A-Z]{2,4}$").expect("static regex"))
}

/// Normalizes English quirks before strftime parsing: "a.m."/"p.m.", "Sept", trailing zone
/// abbreviations (when a `tz` is configured, the abbreviation is redundant).
fn preclean(s: &str, has_tz: bool) -> String {
    let mut t = s
        .trim()
        .replace("a.m.", "am")
        .replace("p.m.", "pm")
        .replace("A.M.", "AM")
        .replace("P.M.", "PM");
    t = t.replace("Sept.", "Sep.").replace("Sept ", "Sep ");
    if has_tz && !t.ends_with("AM") && !t.ends_with("PM") {
        t = tz_abbrev_re().replace(&t, "").into_owned();
    }
    t
}

pub fn parse_datetime(input: &str, spec: &DatetimeSpec, now: DateTime<Utc>) -> Option<ParsedDate> {
    let s = preclean(input, spec.tz.is_some());
    if s.is_empty() {
        return None;
    }
    for f in &spec.formats {
        if let Some(p) = parse_with_format(&s, f) {
            return Some(p);
        }
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        let n: i64 = s.parse().ok()?;
        let dt = match s.len() {
            10 => DateTime::from_timestamp(n, 0),
            13 => DateTime::from_timestamp_millis(n),
            _ => None,
        }?;
        return Some(ParsedDate::Zoned(dt.fixed_offset()));
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(&s) {
        return Some(ParsedDate::Zoned(d));
    }
    for f in ZONED_FORMATS {
        if let Ok(d) = DateTime::parse_from_str(&s, f) {
            return Some(ParsedDate::Zoned(d));
        }
    }
    for f in NAIVE_FORMATS {
        if let Ok(n) = NaiveDateTime::parse_from_str(&s, f) {
            return Some(ParsedDate::Naive(n));
        }
    }
    for f in DATE_FORMATS {
        if let Ok(d) = NaiveDate::parse_from_str(&s, f) {
            return Some(ParsedDate::Date(d));
        }
    }
    if let Ok(d) = DateTime::parse_from_rfc2822(&s) {
        return Some(ParsedDate::Zoned(d));
    }
    if let Some(c) = zh_date_re().captures(&s) {
        let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
        let date = NaiveDate::from_ymd_opt(num(1)? as i32, num(2)?, num(3)?)?;
        return Some(match (num(4), num(5)) {
            (Some(h), Some(m)) => ParsedDate::Naive(date.and_hms_opt(h, m, num(6).unwrap_or(0))?),
            _ => ParsedDate::Date(date),
        });
    }
    if spec.relative {
        return parse_relative(&s, spec.tz, now);
    }
    None
}

fn parse_with_format(s: &str, f: &str) -> Option<ParsedDate> {
    if let Ok(d) = DateTime::parse_from_str(s, f) {
        return Some(ParsedDate::Zoned(d));
    }
    if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
        return Some(ParsedDate::Naive(n));
    }
    NaiveDate::parse_from_str(s, f).ok().map(ParsedDate::Date)
}

fn parse_relative(s: &str, tz: Option<Tz>, now: DateTime<Utc>) -> Option<ParsedDate> {
    static AGO: OnceLock<Regex> = OnceLock::new();
    static ZH_AGO: OnceLock<Regex> = OnceLock::new();
    static DAY_AT: OnceLock<Regex> = OnceLock::new();
    let ago = AGO.get_or_init(|| {
        // Long and compact forms: "3 hours ago", "3h ago", "2mo ago", "3y ago" (YouTube, X, Reddit).
        Regex::new(r"(?i)^(\d+)\s*(second|sec|s|minute|min|m|hour|hr|h|day|d|week|wk|w|month|mo|year|yr|y)s?\s+ago$")
            .expect("static regex")
    });
    let zh_ago =
        ZH_AGO.get_or_init(|| Regex::new(r"^(\d+)\s*(秒|分钟|小时|天|周|个月|月|年)前$").expect("static regex"));
    let day_at = DAY_AT.get_or_init(|| {
        Regex::new(r"(?i)^(yesterday|today|昨天|今天)\s*(?:at\s*)?(\d{1,2})[:：](\d{2})$").expect("static regex")
    });
    let lower = s.trim().to_lowercase();
    if lower == "just now" || lower == "刚刚" {
        return Some(ParsedDate::Zoned(now.fixed_offset()));
    }
    let unit_secs = |u: &str| -> Option<i64> {
        Some(match u {
            "second" | "sec" | "s" | "秒" => 1,
            "minute" | "min" | "m" | "分钟" => 60,
            "hour" | "hr" | "h" | "小时" => 3600,
            "day" | "d" | "天" => 86_400,
            "week" | "wk" | "w" | "周" => 604_800,
            // Months and years are approximate (30 / 365 days): pages only show them for old items.
            "month" | "mo" | "个月" | "月" => 2_592_000,
            "year" | "yr" | "y" | "年" => 31_536_000,
            _ => return None,
        })
    };
    for re in [ago, zh_ago] {
        if let Some(c) = re.captures(&lower) {
            let n: i64 = c[1].parse().ok()?;
            let secs = unit_secs(&c[2])?;
            // Whole seconds: a relative date is approximate, sub-second digits from `now` are noise.
            // Checked: "300000000 years ago" on a hostile page must not panic.
            let delta = Duration::try_seconds(n.checked_mul(secs)?)?;
            let at = now.checked_sub_signed(delta)?.trunc_subsecs(0);
            return Some(ParsedDate::Zoned(at.fixed_offset()));
        }
    }
    let local_now = match tz {
        Some(z) => now.with_timezone(&z).date_naive(),
        None => now.date_naive(),
    };
    if let Some(d) = month_day_this_year(&lower, local_now) {
        return Some(ParsedDate::Date(d));
    }
    if let Some(c) = day_at.captures(&lower) {
        let day = match &c[1] {
            "yesterday" | "昨天" => local_now.pred_opt()?,
            _ => local_now,
        };
        let t = NaiveTime::from_hms_opt(c[2].parse().ok()?, c[3].parse().ok()?, 0)?;
        return Some(ParsedDate::Naive(day.and_time(t)));
    }
    None
}

/// A month and day without a year ("sep 23", "sept. 23", "7月13日"), as feeds show dates of the current
/// year: the most recent such date on or before `today`.
fn month_day_this_year(lower: &str, today: NaiveDate) -> Option<NaiveDate> {
    static EN: OnceLock<Regex> = OnceLock::new();
    static ZH: OnceLock<Regex> = OnceLock::new();
    let en = EN.get_or_init(|| Regex::new(r"^([a-z]{3,9})\.?\s+(\d{1,2})$").expect("static regex"));
    let zh = ZH.get_or_init(|| Regex::new(r"^(\d{1,2})月(\d{1,2})日$").expect("static regex"));
    let (month, day) = if let Some(c) = en.captures(lower) {
        // The word must be a month name or its abbreviation ("sept" too), not any word starting with one
        // ("marathon 26").
        const MONTHS: [&str; 12] = [
            "january",
            "february",
            "march",
            "april",
            "may",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
        ];
        let name = &c[1];
        let m = MONTHS.iter().position(|full| {
            *full == name || (name.len() == 3 && full.starts_with(name)) || (name == "sept" && *full == "september")
        })? as u32
            + 1;
        (m, c[2].parse().ok()?)
    } else {
        let c = zh.captures(lower)?;
        (c[1].parse().ok()?, c[2].parse().ok()?)
    };
    // The latest such date on or before today; Feb 29 looks back to the last leap year.
    (0..8)
        .filter_map(|back| NaiveDate::from_ymd_opt(today.year() - back, month, day))
        .find(|d| *d <= today)
}

// ---------------------------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------------------------

/// An exact decimal: sign, integer digits, fraction digits (no exponent, no float rounding).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal {
    pub negative: bool,
    pub int: String,
    pub frac: String,
}

impl Decimal {
    fn normalized(mut self) -> Decimal {
        let int = self.int.trim_start_matches('0');
        self.int = if int.is_empty() { "0".into() } else { int.into() };
        self.frac = self.frac.trim_end_matches('0').to_string();
        if self.int == "0" && self.frac.is_empty() {
            self.negative = false;
        }
        self
    }

    /// Multiply by 10^power by moving the decimal point.
    fn shift(self, power: usize) -> Decimal {
        let mut digits = self.int.clone();
        let mut frac = self.frac.clone();
        while frac.len() < power {
            frac.push('0');
        }
        digits.push_str(&frac[..power]);
        Decimal {
            negative: self.negative,
            int: digits,
            frac: frac[power..].to_string(),
        }
        .normalized()
    }

    pub fn is_integral(&self) -> bool {
        self.frac.is_empty()
    }

    pub fn as_i128(&self) -> Option<i128> {
        if !self.is_integral() {
            return None;
        }
        let v: i128 = self.int.parse().ok()?;
        Some(if self.negative { -v } else { v })
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.to_string().parse().ok()
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negative {
            write!(f, "-")?;
        }
        write!(f, "{}", self.int)?;
        if !self.frac.is_empty() {
            write!(f, ".{}", self.frac)?;
        }
        Ok(())
    }
}

fn decimal_comma_locale(locale: &str) -> bool {
    let l = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    matches!(
        l.as_str(),
        "de" | "fr" | "es" | "it" | "pt" | "ru" | "nl" | "pl" | "tr" | "id" | "sv" | "da" | "nb" | "fi" | "cs"
    )
}

/// Parse the first number in `s` (engine spec §8.5).
pub fn parse_number(s: &str, locale: &str, cjk_units: bool) -> Option<Decimal> {
    // Grouping is only recognized as separators before exact 3-digit groups, so "Oct 12, 2026" is 12 and
    // "Top 10 2026" is 10. A magnitude letter must touch the digits and not start a word ("2 T-shirts",
    // "10 B.C."); CJK units may follow a space.
    static EN: OnceLock<Regex> = OnceLock::new();
    static EU: OnceLock<Regex> = OnceLock::new();
    let comma = decimal_comma_locale(locale);
    let re = if comma {
        EU.get_or_init(|| {
            Regex::new(r"([-+−]?)(\d{1,3}(?:[.\u{00A0}\u{202F} ']\d{3})+(?:,\d+)?|\d+(?:,\d+)?)(?:([kKmMbBT])(?:$|[^A-Za-z\-])|\s*(万亿|万|亿))?")
                .expect("static regex")
        })
    } else {
        EN.get_or_init(|| {
            Regex::new(r"([-+−]?)(\d{1,3}(?:[,\u{00A0}\u{202F}']\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)(?:([kKmMbBT])(?:$|[^A-Za-z\-])|\s*(万亿|万|亿))?")
                .expect("static regex")
        })
    };
    let c = re.captures(s)?;
    let negative = matches!(c.get(1).map(|m| m.as_str()), Some("-") | Some("−"));
    let body = c.get(2)?.as_str();
    let dec_sep = if comma { ',' } else { '.' };
    let cleaned: String = body.chars().filter(|c| c.is_ascii_digit() || *c == dec_sep).collect();
    let (int, frac) = match cleaned.split_once(dec_sep) {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (cleaned.clone(), String::new()),
    };
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value = Decimal { negative, int, frac }.normalized();
    let power = match (c.get(3).map(|m| m.as_str()), c.get(4).map(|m| m.as_str())) {
        (Some("k" | "K"), _) => 3,
        (Some("m" | "M"), _) => 6,
        (Some("b" | "B"), _) => 9,
        (Some("T"), _) => 12,
        (_, Some("万亿")) if cjk_units => 12,
        (_, Some("万")) if cjk_units => 4,
        (_, Some("亿")) if cjk_units => 8,
        _ => 0,
    };
    Some(if power > 0 { value.shift(power) } else { value })
}

// ---------------------------------------------------------------------------------------------
// URLs
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct UrlSpec {
    pub strip_query: Vec<String>,
    pub keep_fragment: bool,
}

/// Whole seconds from an ISO 8601 duration ("PT1H2M3S", "P1DT2H", "PT3M33.5S") or a clock duration
/// ("1:02:03", "3:33"). Anything else is a miss.
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('P').or_else(|| s.strip_prefix('p')) {
        let (date, time) = match rest.split_once(['T', 't']) {
            Some((d, t)) => (d, t),
            None => (rest, ""),
        };
        let mut total = 0f64;
        let mut seen = false;
        for (part, units) in [
            (date, &[('W', 604_800.0), ('D', 86_400.0)][..]),
            (time, &[('H', 3600.0), ('M', 60.0), ('S', 1.0)][..]),
        ] {
            let mut num = String::new();
            for c in part.chars() {
                if c.is_ascii_digit() || c == '.' || c == ',' {
                    num.push(if c == ',' { '.' } else { c });
                    continue;
                }
                let factor = units.iter().find(|(u, _)| u.eq_ignore_ascii_case(&c))?.1;
                total += num.parse::<f64>().ok()? * factor;
                num.clear();
                seen = true;
            }
            if !num.is_empty() {
                return None;
            }
        }
        return (seen && total.is_finite() && total < 1e15).then_some(total.floor() as u64);
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2
        || parts.len() > 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    // "1:02:03": every field after the first is minutes or seconds, so below 60.
    let mut total = 0u64;
    for (i, p) in parts.iter().enumerate() {
        let v = p.parse::<u64>().ok()?;
        if i > 0 && v >= 60 {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(v)?;
    }
    Some(total)
}

/// Decode percent / base64 / base64url text; None for invalid input or non-UTF-8 output.
pub fn decode(s: &str, kind: pb::DecodeKind) -> Option<String> {
    use base64::Engine as _;
    let bytes = match kind {
        pb::DecodeKind::Percent => percent_encoding::percent_decode_str(&s.replace('+', " ")).collect::<Vec<u8>>(),
        pb::DecodeKind::Base64 => base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(s.trim().trim_end_matches('='))
            .ok()?,
        pb::DecodeKind::Base64url => base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s.trim().trim_end_matches('='))
            .ok()?,
        pb::DecodeKind::Unspecified => return None,
    };
    String::from_utf8(bytes).ok()
}

fn glob_match(pattern: &str, s: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern.eq_ignore_ascii_case(s),
        Some((pre, post)) => {
            let ls = s.to_ascii_lowercase();
            ls.len() >= pre.len() + post.len()
                && ls.starts_with(&pre.to_ascii_lowercase())
                && ls.ends_with(&post.to_ascii_lowercase())
        }
    }
}

/// Resolve against `base`, keep only http(s), drop the fragment unless asked, strip query params.
pub fn normalize_url(raw: &str, base: &Url, spec: &UrlSpec) -> Option<String> {
    let mut u = base.join(raw.trim()).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    if !spec.keep_fragment {
        u.set_fragment(None);
    }
    if !spec.strip_query.is_empty() && u.query().is_some() {
        let kept: Vec<(String, String)> = u
            .query_pairs()
            .filter(|(k, _)| !spec.strip_query.iter().any(|p| glob_match(p, k)))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if kept.is_empty() {
            u.set_query(None);
        } else {
            u.query_pairs_mut().clear().extend_pairs(kept);
        }
    }
    Some(u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_relative_dates_parse() {
        // Arrange
        let spec = DatetimeSpec {
            relative: true,
            ..Default::default()
        };
        let inputs = [
            "3y ago",
            "2mo ago",
            "5 days ago",
            "4h ago",
            "2 months ago",
            "3个月前",
            "1年前",
        ];

        // Act
        let got: Vec<String> = inputs
            .iter()
            .map(|s| match parse_datetime(s, &spec, now()) {
                Some(ParsedDate::Zoned(d)) => d.date_naive().to_string(),
                other => format!("{other:?}"),
            })
            .collect();

        // Assert
        assert_eq!(
            got,
            [
                "2023-10-13",
                "2026-08-13",
                "2026-10-07",
                "2026-10-12",
                "2026-08-13",
                "2026-07-14",
                "2025-10-12"
            ]
        );
    }

    #[test]
    fn month_day_without_year_is_the_latest_past_occurrence() {
        // Arrange
        let spec = DatetimeSpec {
            relative: true,
            ..Default::default()
        };
        let inputs = ["Sep 23", "Sept. 23", "Dec 1", "7月13日", "12月25日"];

        // Act
        let got: Vec<String> = inputs
            .iter()
            .map(|s| match parse_datetime(s, &spec, now()) {
                Some(ParsedDate::Date(d)) => d.to_string(),
                other => format!("{other:?}"),
            })
            .collect();

        // Assert
        assert_eq!(
            got,
            ["2026-09-23", "2026-09-23", "2025-12-01", "2026-07-13", "2025-12-25"]
        );
    }

    #[test]
    fn separate_numbers_and_words_are_not_merged_into_one() {
        // Arrange
        let inputs = [
            ("Oct 12, 2026", "en"),
            ("Top 10 2026 picks", "en"),
            ("2 T-shirts", "en"),
            ("10 B.C.", "en"),
            ("1,234,567.89", "en"),
            ("$1.2B raised", "en"),
            ("1.234,56 €", "de"),
            ("12.5万", "en"),
        ];

        // Act
        let got: Vec<String> = inputs
            .iter()
            .map(|(s, loc)| parse_number(s, loc, true).map(|d| d.to_string()).unwrap_or_default())
            .collect();

        // Assert
        assert_eq!(
            got,
            ["12", "10", "2", "10", "1234567.89", "1200000000", "1234.56", "125000"]
        );
    }

    #[test]
    fn month_day_needs_a_real_month_name_and_handles_leap_days() {
        // Arrange
        let spec = DatetimeSpec {
            relative: true,
            ..Default::default()
        };

        // Act
        let got: Vec<String> = ["marathon 26", "decade 5", "Feb 29", "september 3", "Sept 3"]
            .iter()
            .map(|s| match parse_datetime(s, &spec, now()) {
                Some(ParsedDate::Date(d)) => d.to_string(),
                other => format!("{other:?}"),
            })
            .collect();

        // Assert
        assert_eq!(got, ["None", "None", "2024-02-29", "2026-09-03", "2026-09-03"]);
    }

    #[test]
    fn wall_time_in_a_dst_gap_is_shifted_forward() {
        // Arrange: 02:30 does not exist in New York on 2026-03-08
        let n = NaiveDate::from_ymd_opt(2026, 3, 8)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();

        // Act
        let utc = local_to_utc(n, Some("America/New_York".parse().unwrap()));

        // Assert: 03:30 EDT
        assert_eq!(
            utc.map(|d| d.to_rfc3339()),
            Some("2026-03-08T07:30:00+00:00".to_string())
        );
    }

    #[test]
    fn huge_relative_and_clock_values_are_rejected_without_panicking() {
        // Arrange
        let spec = DatetimeSpec {
            relative: true,
            ..Default::default()
        };

        // Act
        let relative = ["300000 years ago", "99999999999999999 years ago"].map(|s| parse_datetime(s, &spec, now()));
        let clocks = ["307445734561825861:00", "3:99", "1:02:03"].map(parse_duration);

        // Assert
        assert_eq!(relative, [None, None]);
        assert_eq!(clocks, [None, None, Some(3723)]);
    }

    #[test]
    fn magnitude_suffixes_scale_numbers() {
        // Arrange
        let inputs = [
            ("3.68T", false),
            ("52.3M", false),
            ("1.2万亿", true),
            ("2.15亿", true),
            ("5 tons", false),
        ];

        // Act
        let got: Vec<String> = inputs
            .iter()
            .map(|(s, cjk)| parse_number(s, "en", *cjk).map(|d| d.to_string()).unwrap_or_default())
            .collect();

        // Assert
        assert_eq!(got, ["3680000000000", "52300000", "1200000000000", "215000000", "5"]);
    }

    #[test]
    fn meridiem_survives_zone_abbreviation_stripping() {
        // Arrange
        let spec = DatetimeSpec {
            formats: vec!["%b %d, %Y, %I:%M %p".into()],
            tz: Some("Asia/Shanghai".parse().unwrap()),
            ..Default::default()
        };

        // Act
        let got = parse_datetime("Oct 14, 2026, 11:15 AM", &spec, now());

        // Assert
        let expected = NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(11, 15, 0)
            .unwrap();
        assert_eq!(got, Some(ParsedDate::Naive(expected)));
    }

    #[test]
    fn decode_unwraps_redirect_targets() {
        // Arrange
        let bing = "aHR0cHM6Ly9ydXN0LWxhbmcub3JnLw"; // base64url("https://rust-lang.org/"), unpadded
        let google = "https%3A%2F%2Fexample.com%2Fa%3Fb%3D1";

        // Act
        let got = (
            decode(bing, pb::DecodeKind::Base64url),
            decode(google, pb::DecodeKind::Percent),
            decode("not base64!", pb::DecodeKind::Base64),
        );

        // Assert
        assert_eq!(got.0.as_deref(), Some("https://rust-lang.org/"));
        assert_eq!(got.1.as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(got.2, None);
    }

    #[test]
    fn durations_parse_from_iso_and_clock_forms() {
        // Arrange
        let cases = [
            ("PT3M33S", Some(213)),
            ("PT1H2M3S", Some(3723)),
            ("P1DT2H", Some(93_600)),
            ("PT59.9S", Some(59)),
            ("3:33", Some(213)),
            ("1:02:03", Some(3723)),
            ("PT", None),
            ("3 min", None),
            ("1:2:3:4", None),
        ];

        // Act
        let got: Vec<Option<u64>> = cases.iter().map(|(s, _)| parse_duration(s)).collect();

        // Assert
        assert_eq!(got, cases.iter().map(|(_, w)| *w).collect::<Vec<_>>());
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-12T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn builtin_date_formats() {
        // Arrange
        let spec = DatetimeSpec::default();
        let cases = [
            ("2026-10-11T13:30:00Z", "2026-10-11T13:30:00+00:00"),
            ("2026-10-11T09:30:00-04:00", "2026-10-11T13:30:00+00:00"),
            ("2026-10-11 13:30", "2026-10-11T13:30:00+00:00"),
            ("2026-10-11", "2026-10-11T00:00:00+00:00"),
            ("Sun, 11 Oct 2026 13:30:00 +0000", "2026-10-11T13:30:00+00:00"),
            ("1791725400", "2026-10-11T13:30:00+00:00"),
            ("1791725400000", "2026-10-11T13:30:00+00:00"),
            ("2026年10月11日 13:30", "2026-10-11T13:30:00+00:00"),
        ];
        for (input, want) in cases {
            // Act
            let got = parse_datetime(input, &spec, now())
                .and_then(|p| p.to_utc(None))
                .map(|d| d.to_rfc3339());

            // Assert
            assert_eq!(got.as_deref(), Some(want), "input {input}");
        }
    }

    #[test]
    fn custom_format_with_zone_and_abbreviation() {
        // Arrange
        let spec = DatetimeSpec {
            formats: vec!["%b. %d, %Y %I:%M %p".into()],
            tz: Some(chrono_tz::America::New_York),
            ..Default::default()
        };

        // Act
        let got = parse_datetime("Oct. 11, 2026 9:30 a.m. ET", &spec, now()).and_then(|p| p.to_utc(spec.tz));

        // Assert
        assert_eq!(got.unwrap().to_rfc3339(), "2026-10-11T13:30:00+00:00");
    }

    #[test]
    fn relative_dates_need_the_flag() {
        // Arrange
        let off = DatetimeSpec::default();
        let on = DatetimeSpec {
            relative: true,
            ..Default::default()
        };

        // Act
        let without = parse_datetime("3小时前", &off, now());
        let with = parse_datetime("3小时前", &on, now()).and_then(|p| p.to_utc(None));

        // Assert
        assert!(without.is_none());
        assert_eq!(with.unwrap().to_rfc3339(), "2026-10-12T05:00:00+00:00");
    }

    #[test]
    fn numbers_with_grouping_suffixes_and_cjk_units() {
        // Arrange
        let cases = [
            ("1,234 points", "en", false, "1234"),
            ("1.234,5", "de", false, "1234.5"),
            ("1.5k", "en", false, "1500"),
            ("3.2M views", "en", false, "3200000"),
            ("1.2万", "zh", true, "12000"),
            ("3亿", "zh", true, "300000000"),
            ("-0.50", "en", false, "-0.5"),
            ("19.990", "en", false, "19.99"),
        ];
        for (input, locale, cjk, want) in cases {
            // Act
            let got = parse_number(input, locale, cjk).map(|d| d.to_string());

            // Assert
            assert_eq!(got.as_deref(), Some(want), "input {input}");
        }
    }

    #[test]
    fn url_normalization_strips_tracking_and_fragment() {
        // Arrange
        let base = Url::parse("https://news.example/a/b").unwrap();
        let spec = UrlSpec {
            strip_query: vec!["utm_*".into(), "fbclid".into()],
            keep_fragment: false,
        };

        // Act
        let got = normalize_url("../c?id=7&utm_source=x&fbclid=y#top", &base, &spec);
        let rejected = normalize_url("javascript:alert(1)", &base, &spec);

        // Assert
        assert_eq!(got.as_deref(), Some("https://news.example/c?id=7"));
        assert!(rejected.is_none());
    }
}
