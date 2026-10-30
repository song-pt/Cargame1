// 所有場景著色器共用：全域常數、綁定、地形取樣、大氣、霧、陰影、光照。

struct Globals {
    view_proj: mat4x4f,
    inv_view_proj: mat4x4f,
    shadow_mat: mat4x4f,
    cam_pos: vec4f,      // xyz 世界座標，w = 時間（秒）
    light_dir: vec4f,    // 指向主光源（太陽或月亮），w = 夜晚係數 0..1
    light_color: vec4f,  // rgb，w = 曝光
    sun_dir: vec4f,      // 真實太陽方向（天空用），w = 月光強度
    ambient: vec4f,      // w = 世界邊長
    fog: vec4f,          // x 密度，y 高度衰減，z 雲量，w 陰影範圍
    screen: vec4f,       // 寬，高，1/寬，1/高
    grid: vec4f,         // x 精細格距，y 精細格數，z 宏觀格距，w 宏觀格數
};

@group(0) @binding(0) var<uniform> G: Globals;
@group(0) @binding(1) var height_tex: texture_2d<f32>;
@group(0) @binding(2) var water_tex: texture_2d<f32>;
@group(0) @binding(3) var cover_tex: texture_2d<f32>;
@group(0) @binding(4) var road_tex: texture_2d<f32>;
@group(0) @binding(5) var sky_lut: texture_2d<f32>;
@group(0) @binding(6) var lin_samp: sampler;
@group(0) @binding(7) var shadow_tex: texture_depth_2d;
@group(0) @binding(8) var shadow_samp: sampler_comparison;

const PI: f32 = 3.14159265;

fn load_bilinear(t: texture_2d<f32>, xz: vec2f, cell: f32, n: f32) -> f32 {
    let f = xz / cell - 0.5;
    let i = floor(f);
    let t2 = f - i;
    let m = i32(n) - 1;
    let x0 = clamp(i32(i.x), 0, m);
    let z0 = clamp(i32(i.y), 0, m);
    let x1 = clamp(i32(i.x) + 1, 0, m);
    let z1 = clamp(i32(i.y) + 1, 0, m);
    let a = textureLoad(t, vec2i(x0, z0), 0).r;
    let b = textureLoad(t, vec2i(x1, z0), 0).r;
    let c = textureLoad(t, vec2i(x0, z1), 0).r;
    let d = textureLoad(t, vec2i(x1, z1), 0).r;
    return mix(mix(a, b, t2.x), mix(c, d, t2.x), t2.y);
}

fn terrain_height(xz: vec2f) -> f32 {
    return load_bilinear(height_tex, xz, G.grid.x, G.grid.y);
}

fn water_level(xz: vec2f) -> f32 {
    let s = G.ambient.w;
    if (xz.x < 0.0 || xz.y < 0.0 || xz.x > s || xz.y > s) {
        return 0.0;
    }
    return load_bilinear(water_tex, xz, G.grid.z, G.grid.w);
}

fn terrain_normal(xz: vec2f) -> vec3f {
    let e = G.grid.x * 0.7;
    let hl = terrain_height(xz - vec2f(e, 0.0));
    let hr = terrain_height(xz + vec2f(e, 0.0));
    let hd = terrain_height(xz - vec2f(0.0, e));
    let hu = terrain_height(xz + vec2f(0.0, e));
    return normalize(vec3f(hl - hr, 2.0 * e, hd - hu));
}

// ---------- 噪聲 ----------
fn hash21(p: vec2f) -> f32 {
    var q = fract(p * vec2f(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn hash31(p: vec3f) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn vnoise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2f(1.0, 0.0));
    let c = hash21(i + vec2f(0.0, 1.0));
    let d = hash21(i + vec2f(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm2(p: vec2f) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        s += a * vnoise(q);
        q = q * 2.03 + vec2f(1.7, 9.2);
        a *= 0.5;
    }
    return s;
}

// ---------- 天空查找表 ----------
fn lut_uv(d: vec3f) -> vec2f {
    let u = atan2(d.z, d.x) / (2.0 * PI) + 0.5;
    let e = asin(clamp(d.y, -1.0, 1.0));
    let v = 0.5 - 0.5 * sign(e) * sqrt(abs(e) / (0.5 * PI));
    return vec2f(u, v);
}

fn sky_radiance(d: vec3f) -> vec3f {
    return textureSampleLevel(sky_lut, lin_samp, lut_uv(d), 0.0).rgb;
}

fn sky_ambient(n: vec3f) -> vec3f {
    let up = sky_radiance(vec3f(0.0, 1.0, 0.0));
    let hz = 0.5 * (sky_radiance(normalize(vec3f(n.x, 0.15, n.z) + vec3f(0.001, 0.0, 0.0))) + sky_radiance(vec3f(-G.light_dir.x, 0.1, -G.light_dir.z)));
    let ground = vec3f(0.18, 0.17, 0.15) * (G.light_color.rgb * max(G.light_dir.y, 0.0) * 0.3 + up * 0.5);
    let sky = mix(hz, up, clamp(n.y, 0.0, 1.0));
    return mix(ground, sky, clamp(n.y * 0.5 + 0.5, 0.0, 1.0)) * PI * 0.32;
}

// ---------- 霧（空氣透視） ----------
fn apply_fog(col: vec3f, rel: vec3f) -> vec3f {
    let dist = length(rel);
    let dir = rel / max(dist, 1e-3);
    let falloff = G.fog.y;
    let h0 = G.cam_pos.y;
    let dy = rel.y;
    var integral = exp(-falloff * h0);
    if (abs(dy) > 0.01) {
        integral = integral * (1.0 - exp(-falloff * dy)) / (falloff * dy);
    }
    let amount = 1.0 - exp(-G.fog.x * dist * integral);
    let hz = normalize(vec3f(dir.x, max(dir.y, 0.02), dir.z));
    var fogc = sky_radiance(hz);
    // 順光方向的霧帶一點陽光色
    let sun_amt = pow(max(dot(dir, G.sun_dir.xyz), 0.0), 8.0);
    fogc += G.light_color.rgb * sun_amt * 0.15 * (1.0 - G.light_dir.w);
    return mix(col, fogc, clamp(amount, 0.0, 1.0));
}

// ---------- 陰影 ----------
fn shadow_at(rel: vec3f, n: vec3f) -> f32 {
    let p = rel + n * 0.15;
    let sc = G.shadow_mat * vec4f(p, 1.0);
    let uv = sc.xy * vec2f(0.5, -0.5) + 0.5;
    if (uv.x <= 0.0 || uv.y <= 0.0 || uv.x >= 1.0 || uv.y >= 1.0 || sc.z >= 1.0) {
        return 1.0;
    }
    let texel = 1.0 / 2048.0;
    var s = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            s += textureSampleCompareLevel(shadow_tex, shadow_samp, uv + vec2f(f32(x), f32(y)) * texel, sc.z - 0.0008);
        }
    }
    // 靠近陰影範圍邊緣時淡出
    let edge = smoothstep(0.42, 0.5, max(abs(uv.x - 0.5), abs(uv.y - 0.5)));
    return mix(s / 9.0, 1.0, edge);
}

// ---------- 光照 ----------
fn ggx_spec(n: vec3f, v: vec3f, l: vec3f, rough: f32) -> f32 {
    let h = normalize(v + l);
    let a = max(rough * rough, 0.002);
    let a2 = a * a;
    let nh = max(dot(n, h), 0.0);
    let d = a2 / (PI * pow(nh * nh * (a2 - 1.0) + 1.0, 2.0));
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 0.001);
    let k = a * 0.5;
    let vis = 1.0 / ((nl * (1.0 - k) + k) * (nv * (1.0 - k) + k));
    return d * vis * 0.25 * nl;
}

fn fresnel(f0: f32, cosv: f32) -> f32 {
    return f0 + (1.0 - f0) * pow(1.0 - clamp(cosv, 0.0, 1.0), 5.0);
}

// 通用 PBR：albedo, 法線, 粗糙度, 金屬度
fn shade(albedo: vec3f, n: vec3f, rough: f32, metal: f32, rel: vec3f, shadow: f32) -> vec3f {
    let v = -normalize(rel);
    let l = G.light_dir.xyz;
    let nl = max(dot(n, l), 0.0);
    let f0 = mix(vec3f(0.04), albedo, metal);
    let diff = albedo * (1.0 - metal) * nl / PI;
    let spec = f0 * ggx_spec(n, v, l, rough);
    var c = (diff * PI + spec) * G.light_color.rgb * shadow;
    c += albedo * (1.0 - metal) * sky_ambient(n);
    // 環境反射（粗糙表面衰減）
    let r = reflect(-v, n);
    let fr = fresnel(f0.g, dot(n, v)) * (1.0 - rough) * (1.0 - rough);
    c += sky_radiance(normalize(vec3f(r.x, max(r.y, 0.02), r.z))) * fr * mix(vec3f(1.0), albedo, metal) * 0.8;
    return c;
}
