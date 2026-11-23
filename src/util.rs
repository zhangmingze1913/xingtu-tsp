use chrono::{Duration, FixedOffset, SecondsFormat, TimeZone, Utc};

pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn iso_from_now_minutes(minutes: i64) -> String {
    (Utc::now() + Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn minutes_between(start: &str, end: &str) -> f64 {
    let parse = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|v| v.timestamp_millis())
            .unwrap_or(0)
    };
    (parse(end) - parse(start)) as f64 / 60_000.0
}

pub fn china_today_start_utc() -> String {
    let offset = FixedOffset::east_opt(8 * 3600).expect("valid offset");
    let now_cn = Utc::now().with_timezone(&offset);
    let midnight = offset
        .with_ymd_and_hms(
            chrono::Datelike::year(&now_cn),
            chrono::Datelike::month(&now_cn),
            chrono::Datelike::day(&now_cn),
            0,
            0,
            0,
        )
        .single()
        .expect("valid midnight");
    midnight
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn china_day_start_utc(days_ago: i64) -> String {
    let offset = FixedOffset::east_opt(8 * 3600).expect("valid offset");
    let target = (Utc::now() - Duration::days(days_ago)).with_timezone(&offset);
    let midnight = offset
        .with_ymd_and_hms(
            chrono::Datelike::year(&target),
            chrono::Datelike::month(&target),
            chrono::Datelike::day(&target),
            0,
            0,
            0,
        )
        .single()
        .expect("valid midnight");
    midnight
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn china_day_label(days_ago: i64) -> String {
    let offset = FixedOffset::east_opt(8 * 3600).expect("valid offset");
    let target = (Utc::now() - Duration::days(days_ago)).with_timezone(&offset);
    format!(
        "{:02}-{:02}",
        chrono::Datelike::month(&target),
        chrono::Datelike::day(&target)
    )
}

/// Great-circle distance in meters.
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Ray-casting point-in-polygon. Coordinates are `[lon, lat]`.
pub fn point_in_polygon(lat: f64, lon: f64, polygon: &[[f64; 2]]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (xi, yi) = (polygon[i][0], polygon[i][1]);
        let (xj, yj) = (polygon[j][0], polygon[j][1]);
        let intersects =
            ((yi > lat) != (yj > lat)) && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi);
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn page_bounds(page: Option<i64>, page_size: Option<i64>) -> (i64, i64, i64) {
    let page = page.unwrap_or(1).clamp(1, 100_000);
    let page_size = page_size.unwrap_or(20).clamp(1, 200);
    (page, page_size, (page - 1) * page_size)
}

pub fn validate_coord(lat: f64, lon: f64) -> Result<(), String> {
    if !(-90.0..=90.0).contains(&lat) {
        return Err("纬度必须在 -90 到 90 之间".to_string());
    }
    if !(-180.0..=180.0).contains(&lon) {
        return Err("经度必须在 -180 到 180 之间".to_string());
    }
    Ok(())
}

/// Small deterministic PRNG used by the demo seed and fleet simulator.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_between_shanghai_points_is_reasonable() {
        let d = haversine_m(31.2304, 121.4737, 31.2404, 121.4737);
        assert!((d - 1112.0).abs() < 10.0, "distance was {d}");
    }

    #[test]
    fn point_in_polygon_works() {
        let poly = [
            [121.40, 31.20],
            [121.50, 31.20],
            [121.50, 31.30],
            [121.40, 31.30],
        ];
        assert!(point_in_polygon(31.25, 121.45, &poly));
        assert!(!point_in_polygon(31.35, 121.45, &poly));
    }

    #[test]
    fn page_bounds_are_clamped() {
        assert_eq!(page_bounds(Some(0), Some(1000)), (1, 200, 0));
        assert_eq!(page_bounds(Some(3), Some(10)), (3, 10, 20));
    }
}
