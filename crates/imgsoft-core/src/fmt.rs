//! French-style human-readable numbers shared by the app and the CLI.

/// `1 234 567` → `1,2 Mo`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["octets", "Ko", "Mo", "Go"];
    if n < 1024 {
        return format!("{n} {}", UNITS[0]);
    }
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    format!("{} {}", s.replace('.', ","), UNITS[unit])
}

/// Signed percentage of change: `-87 %`, `+3 %`.
pub fn change(before: u64, after: u64) -> String {
    if before == 0 {
        return "—".into();
    }
    let pct = (after as f64 / before as f64 - 1.0) * 100.0;
    if pct.abs() < 0.5 {
        "0 %".into()
    } else if pct <= -99.0 {
        // "−100 %" would read as "nothing left".
        format!("−{:.1} %", (-pct).min(99.9)).replace('.', ",")
    } else if pct < 0.0 {
        format!("−{:.0} %", -pct)
    } else {
        format!("+{pct:.0} %")
    }
}

pub fn duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0).replace('.', ",")
    } else {
        format!("{} min {:02} s", ms / 60_000, (ms / 1000) % 60)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats() {
        assert_eq!(super::bytes(512), "512 octets");
        assert_eq!(super::bytes(1536), "1,50 Ko");
        assert_eq!(super::bytes(5 * 1024 * 1024 + 300_000), "5,29 Mo");
        assert_eq!(super::change(1000, 130), "−87 %");
        assert_eq!(super::change(1000, 1030), "+3 %");
        assert_eq!(super::change(1_000_000, 1_700), "−99,8 %");
        assert_eq!(super::duration_ms(83_000), "1 min 23 s");
    }
}
