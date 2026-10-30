// 天空：查找表生成（單次散射大氣）與全螢幕天空繪製。

struct SkyGlobals {
    view_proj: mat4x4f,
    inv_view_proj: mat4x4f,
    shadow_mat: mat4x4f,
    cam_pos: vec4f,
    light_dir: vec4f,
    light_color: vec4f,
    sun_dir: vec4f,
    ambient: vec4f,
    fog: vec4f,
    screen: vec4f,
    grid: vec4f,
};
@group(0) @binding(0) var<uniform> S: SkyGlobals;

const PI: f32 = 3.14159265;
const R_PLANET: f32 = 6360e3;
const R_ATMO: f32 = 6420e3;
const BETA_R: vec3f = vec3f(5.8e-6, 13.5e-6, 33.1e-6);
const BETA_M: f32 = 21e-6;
const H_R: f32 = 8000.0;
const H_M: f32 = 1200.0;
const OZONE: vec3f = vec3f(0.65e-6, 1.88e-6, 0.085e-6);

fn ray_sphere(o: vec3f, d: vec3f, r: f32) -> vec2f {
    let b = dot(o, d);
    let c = dot(o, o) - r * r;
    let h = b * b - c;
    if (h < 0.0) {
        return vec2f(-1.0, -1.0);
    }
    let s = sqrt(h);
    return vec2f(-b - s, -b + s);
}

fn optical_depth_light(p: vec3f, l: vec3f) -> vec2f {
    let t = ray_sphere(p, l, R_ATMO).y;
    let steps = 6;
    let ds = t / f32(steps);
    var od = vec2f(0.0);
    for (var i = 0; i < steps; i++) {
        let q = p + l * (f32(i) + 0.5) * ds;
        let h = length(q) - R_PLANET;
        if (h < 0.0) {
            return vec2f(1e9);
        }
        od += vec2f(exp(-h / H_R), exp(-h / H_M)) * ds;
    }
    return od;
}

fn scatter(dir: vec3f, l: vec3f, cam_h: f32) -> vec3f {
    let o = vec3f(0.0, R_PLANET + cam_h, 0.0);
    var t_max = ray_sphere(o, dir, R_ATMO).y;
    let g = ray_sphere(o, dir, R_PLANET);
    if (g.x > 0.0) {
        t_max = g.x;
    }
    let steps = 16;
    let ds = t_max / f32(steps);
    var od_r = 0.0;
    var od_m = 0.0;
    var sum_r = vec3f(0.0);
    var sum_m = vec3f(0.0);
    for (var i = 0; i < steps; i++) {
        let p = o + dir * (f32(i) + 0.5) * ds;
        let h = length(p) - R_PLANET;
        let hr = exp(-h / H_R) * ds;
        let hm = exp(-h / H_M) * ds;
        od_r += hr;
        od_m += hm;
        let odl = optical_depth_light(p, l);
        let tau = BETA_R * (od_r + odl.x) + BETA_M * 1.1 * (od_m + odl.y) + OZONE * (od_r + odl.x);
        let att = exp(-tau);
        sum_r += att * hr;
        sum_m += att * hm;
    }
    let mu = dot(dir, l);
    let pr = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    let gg = 0.76;
    let pm = 3.0 / (8.0 * PI) * ((1.0 - gg * gg) * (1.0 + mu * mu)) / ((2.0 + gg * gg) * pow(1.0 + gg * gg - 2.0 * gg * mu, 1.5));
    return sum_r * BETA_R * pr + sum_m * BETA_M * pm;
}

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4f(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2f(p.x, 1.0 - p.y);
    return o;
}

fn lut_dir(uv: vec2f) -> vec3f {
    let az = (uv.x - 0.5) * 2.0 * PI;
    let s = (0.5 - uv.y) * 2.0;
    let e = sign(s) * s * s * 0.5 * PI;
    return vec3f(cos(e) * cos(az), sin(e), cos(e) * sin(az));
}

@fragment
fn fs_lut(in: VOut) -> @location(0) vec4f {
    let d = lut_dir(in.uv);
    let cam_h = max(S.cam_pos.y, 1.0);
    var c = scatter(d, S.sun_dir.xyz, cam_h) * 20.0;
    // 月光下的天空（極暗、偏藍）
    let moon = -S.sun_dir.xyz;
    c += scatter(d, normalize(vec3f(moon.x, abs(moon.y) + 0.2, moon.z)), cam_h) * 20.0 * S.sun_dir.w;
    // 地平線以下：地面反照
    if (d.y < 0.0) {
        c = mix(c, c * 0.6 + vec3f(0.03, 0.035, 0.04) * max(S.sun_dir.y, 0.0) * 4.0, smoothstep(0.0, -0.1, d.y));
    }
    return vec4f(c, 1.0);
}

// ---------- 全螢幕天空 ----------
@group(0) @binding(1) var lut_tex: texture_2d<f32>;
@group(0) @binding(2) var lut_samp: sampler;

fn lut_uv(d: vec3f) -> vec2f {
    let u = atan2(d.z, d.x) / (2.0 * PI) + 0.5;
    let e = asin(clamp(d.y, -1.0, 1.0));
    let v = 0.5 - 0.5 * sign(e) * sqrt(abs(e) / (0.5 * PI));
    return vec2f(u, v);
}

fn hash21(p: vec2f) -> f32 {
    var q = fract(p * vec2f(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn vnoise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2f(1.0, 0.0)), u.x), mix(hash21(i + vec2f(0.0, 1.0)), hash21(i + vec2f(1.0, 1.0)), u.x), u.y);
}

fn clouds(d: vec3f, t: f32) -> vec2f {
    if (d.y <= 0.01) {
        return vec2f(0.0);
    }
    let p = d.xz / d.y * 1.6 + vec2f(t * 0.004, t * 0.002);
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 5; i++) {
        s += a * vnoise(q);
        q = q * 2.1 + vec2f(3.1, 1.7);
        a *= 0.5;
    }
    let cover = S.fog.z;
    let dens = smoothstep(1.0 - cover, 1.0 - cover + 0.35, s);
    // 粗略自陰影：沿太陽方向偏移再取樣
    let ps = p + S.sun_dir.xz * 0.08;
    let s2 = 0.5 * vnoise(ps) + 0.25 * vnoise(ps * 2.1 + vec2f(3.1, 1.7));
    return vec2f(dens * smoothstep(0.01, 0.15, d.y), s2);
}

@fragment
fn fs_sky(in: VOut) -> @location(0) vec4f {
    let ndc = vec4f(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 0.5, 1.0);
    let w = S.inv_view_proj * ndc;
    let d = normalize(w.xyz / w.w);
    var c = textureSampleLevel(lut_tex, lut_samp, lut_uv(d), 0.0).rgb;
    let night = S.light_dir.w;
    // 太陽圓盤
    let sd = dot(d, S.sun_dir.xyz);
    let sun_vis = smoothstep(0.99996, 0.99999, sd) * smoothstep(-0.02, 0.01, d.y);
    c += S.light_color.rgb * sun_vis * 60.0 * (1.0 - night);
    // 月亮
    let md = dot(d, -S.sun_dir.xyz);
    c += vec3f(0.8, 0.85, 1.0) * smoothstep(0.99985, 0.99992, md) * 3.0 * night;
    // 星星
    if (night > 0.0 && d.y > 0.0) {
        let g = floor(d.xz / (d.y + 0.3) * 260.0);
        let h = hash21(g);
        let star = step(0.997, h) * (0.5 + 0.5 * sin(S.cam_pos.w * 2.0 + h * 50.0));
        c += vec3f(star) * 0.6 * night * smoothstep(0.0, 0.2, d.y);
    }
    // 雲
    let cl = clouds(d, S.cam_pos.w);
    if (cl.x > 0.0) {
        let lit = S.light_color.rgb * (1.1 - cl.y) * 0.6 + textureSampleLevel(lut_tex, lut_samp, lut_uv(vec3f(0.0, 1.0, 0.0)), 0.0).rgb * 1.6;
        let silver = pow(max(sd, 0.0), 12.0) * S.light_color.rgb * 0.8 * (1.0 - night);
        c = mix(c, lit + silver, cl.x * 0.92);
    }
    return vec4f(c, 1.0);
}
