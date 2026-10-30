// 後處理：泛光降採樣 / 升採樣、色調映射。

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var<uniform> P: vec4f; // x 曝光, y 泛光強度, z 是否需要手動 gamma, w 時間

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

fn luma(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

// 13 點降採樣（COD 方法），第一級帶亮度門檻
fn down13(uv: vec2f) -> vec3f {
    let ts = 1.0 / vec2f(textureDimensions(src));
    var c = textureSampleLevel(src, samp, uv, 0.0).rgb * 0.125;
    c += (textureSampleLevel(src, samp, uv + ts * vec2f(-1.0, -1.0), 0.0).rgb + textureSampleLevel(src, samp, uv + ts * vec2f(1.0, -1.0), 0.0).rgb
        + textureSampleLevel(src, samp, uv + ts * vec2f(-1.0, 1.0), 0.0).rgb + textureSampleLevel(src, samp, uv + ts * vec2f(1.0, 1.0), 0.0).rgb) * 0.125;
    c += (textureSampleLevel(src, samp, uv + ts * vec2f(-2.0, -2.0), 0.0).rgb + textureSampleLevel(src, samp, uv + ts * vec2f(2.0, -2.0), 0.0).rgb
        + textureSampleLevel(src, samp, uv + ts * vec2f(-2.0, 2.0), 0.0).rgb + textureSampleLevel(src, samp, uv + ts * vec2f(2.0, 2.0), 0.0).rgb) * 0.03125;
    c += (textureSampleLevel(src, samp, uv + ts * vec2f(-2.0, 0.0), 0.0).rgb + textureSampleLevel(src, samp, uv + ts * vec2f(2.0, 0.0), 0.0).rgb
        + textureSampleLevel(src, samp, uv + ts * vec2f(0.0, -2.0), 0.0).rgb + textureSampleLevel(src, samp, uv + ts * vec2f(0.0, 2.0), 0.0).rgb) * 0.0625;
    return c;
}

@fragment
fn fs_down_first(in: VOut) -> @location(0) vec4f {
    var c = down13(in.uv) * P.x;
    // 柔和門檻，避免螢火蟲
    let l = luma(c);
    c *= smoothstep(0.8, 2.5, l) / (1.0 + l * 0.15);
    return vec4f(c, 1.0);
}

@fragment
fn fs_down(in: VOut) -> @location(0) vec4f {
    return vec4f(down13(in.uv), 1.0);
}

@fragment
fn fs_up(in: VOut) -> @location(0) vec4f {
    let ts = 1.0 / vec2f(textureDimensions(src));
    var c = textureSampleLevel(src, samp, in.uv, 0.0).rgb * 4.0;
    c += (textureSampleLevel(src, samp, in.uv + vec2f(ts.x, 0.0), 0.0).rgb + textureSampleLevel(src, samp, in.uv - vec2f(ts.x, 0.0), 0.0).rgb
        + textureSampleLevel(src, samp, in.uv + vec2f(0.0, ts.y), 0.0).rgb + textureSampleLevel(src, samp, in.uv - vec2f(0.0, ts.y), 0.0).rgb) * 2.0;
    c += textureSampleLevel(src, samp, in.uv + ts, 0.0).rgb + textureSampleLevel(src, samp, in.uv - ts, 0.0).rgb
        + textureSampleLevel(src, samp, in.uv + vec2f(ts.x, -ts.y), 0.0).rgb + textureSampleLevel(src, samp, in.uv + vec2f(-ts.x, ts.y), 0.0).rgb;
    return vec4f(c / 16.0, 1.0);
}

// AgX 風格的色調映射（平滑高光，色彩不過飽和）
fn agx_curve(x: vec3f) -> vec3f {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3f) -> vec3f {
    let m = mat3x3f(
        vec3f(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3f(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3f(0.0792237451477643, 0.0791661274605434, 0.879142973793104));
    let minev = -12.47393;
    let maxev = 4.026069;
    var v = m * c;
    v = clamp(log2(max(v, vec3f(1e-10))), vec3f(minev), vec3f(maxev));
    v = (v - minev) / (maxev - minev);
    v = agx_curve(v);
    // 輕微加對比與飽和（Punchy）
    let l = luma(v);
    v = mix(vec3f(l), v, 1.12);
    let inv = mat3x3f(
        vec3f(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3f(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3f(-0.0990297440797205, -0.0989611768448433, 1.15107367264116));
    return clamp(inv * v, vec3f(0.0), vec3f(1.0));
}

@fragment
fn fs_tonemap(in: VOut) -> @location(0) vec4f {
    var c = textureSampleLevel(src, samp, in.uv, 0.0).rgb * P.x;
    c += textureSampleLevel(bloom, samp, in.uv, 0.0).rgb * P.y;
    // 暗角
    let d = in.uv - 0.5;
    c *= 1.0 - dot(d, d) * 0.35;
    // AgX 輸出的是顯示用（已含 gamma 2.2 近似）的值
    var o = agx(c);
    if (P.z < 0.5) {
        // 目標格式是 sRGB：轉回線性，讓硬體再編碼
        o = pow(o, vec3f(2.2));
    }
    // 抖動，避免天空色帶
    let n = fract(sin(dot(in.pos.xy + P.w, vec2f(12.9898, 78.233))) * 43758.5453) - 0.5;
    o += n / 255.0;
    return vec4f(o, 1.0);
}
