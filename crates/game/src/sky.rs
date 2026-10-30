//! 時間 → 太陽 / 月亮方向與光色。

use glam::Vec3;

pub struct Lighting {
    pub sun_dir: Vec3,
    pub light_dir: Vec3,
    pub light_color: Vec3,
    pub night: f32,
    pub exposure: f32,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// hours: 0..24；緯度約 25°N（東亞），春分
pub fn lighting(hours: f32) -> Lighting {
    let lat = 25.0f32.to_radians();
    let dec = 0.0f32;
    let ha = (hours - 12.0) * 15.0f32.to_radians();
    let sin_el = lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos();
    let el = sin_el.clamp(-1.0, 1.0).asin();
    // 方位角：上午在東（+X），下午在西
    let az = (-ha.sin()).atan2(ha.cos() * lat.sin() - dec.tan() * lat.cos());
    let sun_dir = Vec3::new(el.cos() * az.sin(), sin_el, el.cos() * az.cos() * -1.0).normalize();

    // 大氣透過率（Kasten 氣團）
    let el_deg = el.to_degrees().max(-2.0);
    let airmass = 1.0 / (sin_el.max(0.0) + 0.50572 * (el_deg + 6.07995).powf(-1.6364));
    let tau = Vec3::new(0.0464 + 0.0277, 0.108 + 0.0277, 0.265 + 0.0277);
    let trans = Vec3::new((-tau.x * airmass).exp(), (-tau.y * airmass).exp(), (-tau.z * airmass).exp());
    let day = smoothstep(-0.04, 0.06, sin_el);
    let night = smoothstep(0.02, -0.14, sin_el);
    let (light_dir, light_color) = if sin_el > -0.03 {
        (Vec3::new(sun_dir.x, sun_dir.y.max(0.02), sun_dir.z).normalize(), trans * 3.4 * day)
    } else {
        let m = Vec3::new(-sun_dir.x, (-sun_dir.y).max(0.25), -sun_dir.z).normalize();
        (m, Vec3::new(0.10, 0.12, 0.18) * 0.55 * night)
    };
    let exposure = 1.0 + 7.0 * night + 1.5 * smoothstep(0.25, 0.0, sin_el) * (1.0 - night);
    Lighting { sun_dir, light_dir, light_color, night, exposure }
}
