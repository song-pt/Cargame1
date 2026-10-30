// 地形（CDLOD）與水面。common.wgsl 會被拼接在前面。

struct Node {
    @location(0) node: vec4f,   // x0, z0, size, 0
    @location(1) morph: vec4f,  // 形變開始距離, 結束距離
};

struct TOut {
    @builtin(position) pos: vec4f,
    @location(0) rel: vec3f,
    @location(1) world_xz: vec2f,
};

const PATCH: f32 = 32.0;

fn cdlod_xz(g: vec2f, nd: Node) -> vec2f {
    var xz = nd.node.xy + g / PATCH * nd.node.z;
    let h = terrain_height(xz);
    let d = distance(vec3f(xz.x, h, xz.y), G.cam_pos.xyz);
    let k = clamp((d - nd.morph.x) / (nd.morph.y - nd.morph.x), 0.0, 1.0);
    let frac = fract(g * 0.5) * 2.0;
    let gm = g - frac * k;
    return nd.node.xy + gm / PATCH * nd.node.z;
}

@vertex
fn vs_terrain(@location(2) g: vec2f, nd: Node) -> TOut {
    let xz = cdlod_xz(g, nd);
    let h = terrain_height(xz);
    let rel = vec3f(xz.x, h, xz.y) - G.cam_pos.xyz;
    var o: TOut;
    o.pos = G.view_proj * vec4f(rel, 1.0);
    o.rel = rel;
    o.world_xz = xz;
    return o;
}

fn grass_color(xz: vec2f, h: f32) -> vec3f {
    let n1 = fbm2(xz * 0.013);
    let n2 = vnoise(xz * 0.11);
    let dry = smoothstep(0.45, 0.75, n1);
    var c = mix(vec3f(0.16, 0.26, 0.07), vec3f(0.30, 0.34, 0.12), dry);
    c *= 0.85 + 0.3 * n2;
    c = mix(c, vec3f(0.22, 0.27, 0.12), smoothstep(150.0, 450.0, h) * 0.5);
    return c;
}

@fragment
fn fs_terrain(in: TOut) -> @location(0) vec4f {
    let xz = in.world_xz;
    let wpos = in.rel + G.cam_pos.xyz;
    let h = wpos.y;
    var n = terrain_normal(xz);
    let dist = length(in.rel);
    // 細節法線
    let dn = vec2f(vnoise(xz * 0.9) - 0.5, vnoise(xz * 0.9 + vec2f(17.0, 3.0)) - 0.5) * 0.35 * (1.0 - smoothstep(30.0, 200.0, dist));
    n = normalize(n + vec3f(dn.x, 0.0, dn.y));
    let slope = 1.0 - n.y;
    let cuv = xz / (G.grid.z * G.grid.w);
    let cov = textureSampleLevel(cover_tex, lin_samp, cuv, 0.0);
    let road = load_bilinear(road_tex, xz, G.grid.x, G.grid.y);

    var albedo = grass_color(xz, h);
    var rough = 0.9;
    // 森林地面（樹木在 M1 加入）
    let forest_col = vec3f(0.07, 0.13, 0.05) * (0.8 + 0.4 * vnoise(xz * 0.05));
    albedo = mix(albedo, forest_col, cov.r * 0.9);
    // 特殊地表
    let special = cov.a;
    if (special > 0.9) {
        // 花田：彩色小點，遠處退化成平均色
        let cell = floor(xz * 1.6);
        let hsh = hash21(cell);
        let fade = 1.0 - smoothstep(40.0, 160.0, dist);
        var petal = vec3f(0.85, 0.35, 0.55);
        if (hsh > 0.66) { petal = vec3f(0.95, 0.85, 0.25); } else if (hsh > 0.33) { petal = vec3f(0.55, 0.45, 0.85); }
        let dotm = step(0.55, vnoise(xz * 3.1)) * fade;
        albedo = mix(albedo, mix(albedo, petal, 0.35), (special - 0.9) * 10.0);
        albedo = mix(albedo, petal, dotm * (special - 0.9) * 10.0 * 0.8);
    } else if (special > 0.55) {
        // 茶園：沿等高線的茶樹行
        let rows = abs(fract(h / 1.7 + vnoise(xz * 0.02) * 0.6) - 0.5) * 2.0;
        let bush = vec3f(0.10, 0.24, 0.07);
        let soil = vec3f(0.24, 0.18, 0.11);
        let t = (special - 0.55) / 0.35;
        albedo = mix(albedo, mix(bush, soil, smoothstep(0.55, 0.8, rows)), clamp(t, 0.0, 1.0));
    } else if (special > 0.2) {
        // 城市地面：混凝土 / 人行道
        let t = clamp((special - 0.2) / 0.2, 0.0, 1.0);
        let tile = step(0.04, fract(xz.x * 0.5)) * step(0.04, fract(xz.y * 0.5));
        albedo = mix(albedo, vec3f(0.33, 0.32, 0.30) * (0.85 + 0.15 * tile) * (0.9 + 0.2 * vnoise(xz * 0.3)), t);
        rough = mix(rough, 0.8, t);
    }
    // 沙灘
    albedo = mix(albedo, vec3f(0.62, 0.55, 0.40) * (0.9 + 0.2 * vnoise(xz * 0.4)), cov.b);
    // 岩石
    let strata = 0.8 + 0.25 * vnoise(vec2f(h * 0.35, xz.x * 0.01 + xz.y * 0.01));
    let rock_col = vec3f(0.33, 0.30, 0.27) * strata;
    let rockiness = max(cov.g, smoothstep(0.32, 0.55, slope));
    albedo = mix(albedo, rock_col, rockiness);
    // 高山積雪
    let snow = smoothstep(680.0, 760.0, h + 40.0 * vnoise(xz * 0.01)) * smoothstep(0.45, 0.2, slope);
    albedo = mix(albedo, vec3f(0.85, 0.87, 0.9), snow);
    rough = mix(rough, 0.6, snow);
    // 路肩
    albedo = mix(albedo, vec3f(0.30, 0.26, 0.21), smoothstep(0.0, 0.5, road) * (1.0 - special * 0.5));
    // 水邊濕潤
    let wl = water_level(xz);
    let wet = 1.0 - smoothstep(0.0, 1.5, h - wl);
    albedo *= 1.0 - 0.45 * wet;
    rough = mix(rough, 0.3, wet);

    let sh = shadow_at(in.rel, n);
    var c = shade(albedo, n, rough, 0.0, in.rel, sh);
    c = apply_fog(c, in.rel);
    return vec4f(c, 1.0);
}

// ---------- 水面 ----------
struct WOut {
    @builtin(position) pos: vec4f,
    @location(0) rel: vec3f,
    @location(1) world_xz: vec2f,
};

@vertex
fn vs_water(@location(2) g: vec2f, nd: Node) -> WOut {
    let xz = nd.node.xy + g / PATCH * nd.node.z;
    let y = water_level(xz);
    let rel = vec3f(xz.x, y, xz.y) - G.cam_pos.xyz;
    var o: WOut;
    o.pos = G.view_proj * vec4f(rel, 1.0);
    o.rel = rel;
    o.world_xz = xz;
    return o;
}

// 世界外的遠海：以相機為中心的大網格，只畫世界範圍之外
@vertex
fn vs_ocean(@location(2) g: vec2f) -> WOut {
    let span = 60000.0;
    let c = floor(G.cam_pos.xz / 400.0) * 400.0;
    let xz = c + (g / PATCH - 0.5) * span;
    let rel = vec3f(xz.x, 0.0, xz.y) - G.cam_pos.xyz;
    var o: WOut;
    o.pos = G.view_proj * vec4f(rel, 1.0);
    o.rel = rel;
    o.world_xz = xz;
    return o;
}

fn wave_normal(xz: vec2f, t: f32, dist: f32) -> vec3f {
    var d = vec2f(0.0);
    let dirs = array<vec2f, 4>(vec2f(1.0, 0.3), vec2f(-0.4, 1.0), vec2f(0.7, -0.8), vec2f(-1.0, -0.2));
    let freqs = array<f32, 4>(0.11, 0.23, 0.47, 0.9);
    let amps = array<f32, 4>(0.30, 0.18, 0.10, 0.05);
    for (var i = 0; i < 4; i++) {
        let k = normalize(dirs[i]) * freqs[i];
        let ph = dot(xz, k) * 6.2831 + t * sqrt(9.81 * freqs[i] * 6.2831);
        d += k * cos(ph) * amps[i] * freqs[i] * 6.2831;
    }
    let fine = vec2f(vnoise(xz * 1.3 + t * 0.6) - 0.5, vnoise(xz * 1.3 - t * 0.5 + vec2f(9.0, 2.0)) - 0.5) * 0.25;
    d += fine * (1.0 - smoothstep(20.0, 120.0, dist));
    d *= 1.0 - smoothstep(300.0, 3000.0, dist) * 0.8;
    return normalize(vec3f(-d.x, 1.0, -d.y));
}

fn water_shade(in: WOut, far_ocean: bool) -> vec4f {
    let xz = in.world_xz;
    let s = G.ambient.w;
    let inside = xz.x > 0.0 && xz.y > 0.0 && xz.x < s && xz.y < s;
    if (far_ocean && inside) {
        discard;
    }
    let surf = in.rel.y + G.cam_pos.y;
    var ground = -60.0;
    if (inside) {
        ground = terrain_height(xz);
    }
    let depth = surf - ground;
    if (depth < 0.03) {
        discard;
    }
    let dist = length(in.rel);
    let v = -in.rel / dist;
    let n = wave_normal(xz, G.cam_pos.w, dist);
    let deep = vec3f(0.005, 0.03, 0.05);
    let shallow = vec3f(0.03, 0.14, 0.13);
    let body = mix(shallow, deep, 1.0 - exp(-depth * 0.25));
    let ambient = sky_ambient(vec3f(0.0, 1.0, 0.0));
    var c = body * (ambient + G.light_color.rgb * max(G.light_dir.y, 0.0) * 0.3);
    let r = reflect(-v, n);
    let fr = fresnel(0.02, max(dot(n, v), 0.0));
    let refl = sky_radiance(normalize(vec3f(r.x, max(r.y, 0.01), r.z)));
    let sh = shadow_at(in.rel, vec3f(0.0, 1.0, 0.0));
    let spec = ggx_spec(n, v, G.light_dir.xyz, 0.06) * G.light_color.rgb * sh;
    c = mix(c, refl, fr) + spec;
    // 岸邊泡沫
    let foam = (1.0 - smoothstep(0.05, 0.6, depth)) * step(0.45, vnoise(xz * 0.8 + G.cam_pos.w * 0.3));
    c = mix(c, (ambient + G.light_color.rgb * max(G.light_dir.y, 0.0)) * 0.7, foam * 0.6);
    c = apply_fog(c, in.rel);
    return vec4f(c, 1.0);
}

@fragment
fn fs_water(in: WOut) -> @location(0) vec4f {
    return water_shade(in, false);
}

@fragment
fn fs_ocean(in: WOut) -> @location(0) vec4f {
    return water_shade(in, true);
}
