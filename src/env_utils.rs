use std::env;

pub fn env_bool(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "y" | "on"
        ),
        Err(_) => default,
    }
}

pub fn env_float(name: &str, default: f64) -> f64 {
    match env::var(name) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                default
            } else {
                t.parse::<f64>().unwrap_or(default)
            }
        }
        Err(_) => default,
    }
}

pub fn env_int(name: &str, default: i64) -> i64 {
    match env::var(name) {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                default
            } else if let Ok(i) = t.parse::<i64>() {
                i
            } else if let Ok(f) = t.parse::<f64>() {
                f as i64
            } else {
                default
            }
        }
        Err(_) => default,
    }
}
