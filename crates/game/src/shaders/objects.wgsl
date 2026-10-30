// 道路、建築、車輛。common.wgsl 會被拼接在前面。

// ---------- 道路 ----------
struct RIn {
    @location(0) pos: vec3f,
    @location(1) uv: vec2f,      // x 橫向 -1..1，y 沿路距離（米）
    @location(2) info: vec2f,    // x 道路等級，y 1 = 橋側面
};
struct ROut {
    @builtin(position) pos: vec4f,
    @location(0) rel: vec3f,
    @location(1) uv: vec2f,
    @location(2) info: vec2f,
};

@vertex
fn vs_road(v: RIn) -> ROut {
    let rel = v.pos - G.cam_pos.xyz;
    var o: ROut;
    o.pos = G.view_proj * vec4f(rel, 1.0);
    o.rel = rel;
    o.uv = v.uv;
    o.info = v.info;
    return o;
}

fn dashed(along: f32, on: f32, off: f32) -> f32 {
    return step(fract(along / (on + off)), on / (on + off));
}

@fragment
fn fs_road(in: ROut) -> @location(0) vec4f {
    let rclass = i32(in.info.x + 0.5);
    let wp = in.rel + G.cam_pos.xyz;
    let face_n = normalize(cross(dpdy(in.rel), dpdx(in.rel)));
    if (in.info.y > 0.5) {
        // 橋側面：混凝土
        var c = shade(vec3f(0.45, 0.44, 0.42), face_n, 0.85, 0.0, in.rel, 1.0);
        return vec4f(apply_fog(c, in.rel), 1.0);
    }
    let x = in.uv.x;
    let along = in.uv.y;
    let dist = length(in.rel);
    var albedo = vec3f(0.085, 0.085, 0.09) * (0.85 + 0.3 * vnoise(wp.xz * 2.7)) * (0.9 + 0.2 * vnoise(wp.xz * 0.2));
    var rough = 0.85;
    // 路面修補與油漬
    albedo *= 1.0 - 0.25 * step(0.8, vnoise(wp.xz * 0.15));
    let ax = abs(x);
    var paint = 0.0;
    var yellow = 0.0;
    var half_w = 7.0;
    if (rclass == 0) { half_w = 7.0; } else if (rclass == 1) { half_w = 4.5; } else if (rclass == 2) { half_w = 3.5; } else if (rclass == 3) { half_w = 11.0; } else if (rclass == 4) { half_w = 4.0; } else { half_w = 2.25; }
    let xm = x * half_w; // 米
    let line_w = 0.15 / half_w;
    // 邊線
    if (rclass <= 3) {
        paint = max(paint, step(1.0 - 0.06 - line_w * 2.0, ax) * step(ax, 1.0 - 0.06));
    }
    // 中線
    if (rclass == 0) {
        yellow = step(abs(abs(xm) - 0.15), 0.08);
        // 高速：每側兩車道，車道虛線
        paint = max(paint, step(abs(abs(xm) - 3.5), 0.08) * dashed(along, 4.0, 8.0));
    } else if (rclass == 1 || rclass == 2) {
        yellow = step(abs(xm), 0.08) * select(dashed(along, 4.0, 6.0), 1.0, rclass == 2);
    } else if (rclass == 3) {
        yellow = step(abs(abs(xm) - 0.2), 0.1);
        paint = max(paint, step(abs(abs(xm) - 3.6), 0.08) * dashed(along, 4.0, 6.0));
        paint = max(paint, step(abs(abs(xm) - 7.2), 0.08) * dashed(along, 4.0, 6.0));
    } else if (rclass == 4) {
        paint = step(abs(xm), 0.06) * dashed(along, 3.0, 5.0);
    }
    let fade = 1.0 - smoothstep(150.0, 600.0, dist) * 0.7;
    albedo = mix(albedo, vec3f(0.75, 0.75, 0.72), paint * fade);
    albedo = mix(albedo, vec3f(0.75, 0.55, 0.12), yellow * fade);
    rough = mix(rough, 0.6, max(paint, yellow));
    // 邊緣略暗、與地形融合
    albedo *= 1.0 - 0.3 * smoothstep(0.92, 1.0, ax);
    let n = vec3f(0.0, 1.0, 0.0);
    let sh = shadow_at(in.rel, n);
    var c = shade(albedo, n, rough, 0.0, in.rel, sh);
    c = apply_fog(c, in.rel);
    return vec4f(c, 1.0);
}

// ---------- 建築 ----------
struct BIn {
    @location(0) local: vec3f,   // 單位立方體 -0.5..0.5（y 0..1）
    @location(1) normal: vec3f,
    @location(2) ipos: vec4f,    // xyz, w = 角度
    @location(3) isize: vec4f,   // xyz 尺寸, w = 種子（浮點）
    @location(4) ikind: u32,
};
struct BOut {
    @builtin(position) pos: vec4f,
    @location(0) rel: vec3f,
    @location(1) lp: vec3f,       // 局部座標（米），x,z 置中，y 從地面起
    @location(2) ln: vec3f,       // 局部法線
    @location(3) wn: vec3f,
    @location(4) @interpolate(flat) kind: u32,
    @location(5) @interpolate(flat) seed: f32,
    @location(6) size: vec3f,
};

fn rot_y(a: f32, v: vec3f) -> vec3f {
    let c = cos(a);
    let s = sin(a);
    return vec3f(v.x * c - v.z * s, v.y, v.x * s + v.z * c);
}

@vertex
fn vs_building(v: BIn) -> BOut {
    let lp = v.local * v.isize.xyz;
    let wp = v.ipos.xyz + rot_y(v.ipos.w, lp);
    let rel = wp - G.cam_pos.xyz;
    var o: BOut;
    o.pos = G.view_proj * vec4f(rel, 1.0);
    o.rel = rel;
    o.lp = lp;
    o.ln = v.normal;
    o.wn = rot_y(v.ipos.w, v.normal);
    o.kind = v.ikind;
    o.seed = v.isize.w;
    o.size = v.isize.xyz;
    return o;
}

@vertex
fn vs_building_shadow(v: BIn) -> @builtin(position) vec4f {
    let lp = v.local * v.isize.xyz;
    let wp = v.ipos.xyz + rot_y(v.ipos.w, lp);
    return G.shadow_mat * vec4f(wp - G.cam_pos.xyz, 1.0);
}

fn palette_old(h: f32) -> vec3f {
    let cols = array<vec3f, 6>(
        vec3f(0.62, 0.56, 0.46), vec3f(0.55, 0.55, 0.52), vec3f(0.48, 0.55, 0.50),
        vec3f(0.70, 0.66, 0.58), vec3f(0.45, 0.38, 0.33), vec3f(0.60, 0.50, 0.42));
    return cols[u32(h * 5.99)];
}

fn neon_color(h: f32) -> vec3f {
    let cols = array<vec3f, 6>(
        vec3f(1.0, 0.15, 0.35), vec3f(0.2, 0.9, 1.0), vec3f(1.0, 0.75, 0.15),
        vec3f(0.6, 0.25, 1.0), vec3f(0.2, 1.0, 0.45), vec3f(1.0, 0.35, 0.1));
    return cols[u32(h * 5.99)];
}

@fragment
fn fs_building(in: BOut) -> @location(0) vec4f {
    let kind = in.kind;
    let seed = in.seed;
    let lp = in.lp;
    let sz = in.size;
    let n = normalize(in.wn);
    let night = G.light_dir.w;
    let roof = in.ln.y > 0.5;
    let front = in.ln.z > 0.5;
    // 立面座標：沿牆水平距離 u，高度 y
    var u = lp.x;
    var wall_w = sz.x;
    if (abs(in.ln.x) > 0.5) {
        u = lp.z;
        wall_w = sz.z;
    }
    let y = lp.y;
    var albedo = vec3f(0.5);
    var rough = 0.85;
    var metal = 0.0;
    var emissive = vec3f(0.0);
    var glass = 0.0;

    // 鳥居與涼亭：以挖空做出造型
    if (kind == 6u) {
        let ax = abs(lp.x);
        let pillar = step(abs(ax - 2.1), 0.25);
        let beam1 = step(4.9, y) * step(y, 5.5);
        let beam2 = step(3.9, y) * step(y, 4.25) * step(ax, 2.6);
        if (pillar + beam1 + beam2 < 0.5) { discard; }
        albedo = vec3f(0.62, 0.08, 0.04);
        if (beam1 > 0.5 && y > 5.3) { albedo = vec3f(0.05); }
        let c = shade(albedo, n, 0.6, 0.0, in.rel, shadow_at(in.rel, n));
        return vec4f(apply_fog(c, in.rel), 1.0);
    }
    if (kind == 8u) {
        let corner = step(sz.x * 0.5 - 0.4, abs(lp.x)) * step(sz.z * 0.5 - 0.4, abs(lp.z));
        let is_roof = step(sz.y - 0.8, y);
        let floor_ = step(y, 0.4);
        if (corner + is_roof + floor_ < 0.5) { discard; }
        albedo = select(vec3f(0.45, 0.32, 0.22), vec3f(0.15, 0.17, 0.18), is_roof > 0.5);
        let c = shade(albedo, n, 0.7, 0.0, in.rel, shadow_at(in.rel, n));
        return vec4f(apply_fog(c, in.rel), 1.0);
    }

    let floor_h = select(3.1, 3.0, kind == 1u);
    let fl = floor(y / floor_h);
    let fy = fract(y / floor_h);
    let bay_w = select(2.6, 1.6, kind == 2u || kind == 9u);
    let bays = max(floor(wall_w / bay_w), 1.0);
    let bu = (u / wall_w + 0.5) * bays;
    let bi = floor(bu);
    let bx = fract(bu);
    let win_rand = hash31(vec3f(bi, fl, seed * 0.001 + f32(in.ln.x * 3.0 + in.ln.z * 7.0)));
    var is_win = step(0.18, bx) * step(bx, 0.82) * step(0.25, fy) * step(fy, 0.82);

    if (kind == 0u || kind == 4u) {
        // 老城店屋：磁磚外牆，窗框，頂樓加蓋，招牌
        albedo = palette_old(fract(seed * 0.000123));
        albedo *= 0.9 + 0.15 * vnoise(vec2f(u, y) * 3.0);
        // 髒污：雨水痕
        albedo *= 1.0 - 0.18 * smoothstep(0.4, 0.9, vnoise(vec2f(u * 0.7, y * 0.08)));
        if (fl < 1.0 && kind == 0u) {
            // 一樓店面：大玻璃或鐵捲門
            is_win = step(0.08, bx) * step(bx, 0.92) * step(0.05, fy) * step(fy, 0.75);
            if (win_rand > 0.6) {
                albedo = vec3f(0.45, 0.46, 0.47) * (0.8 + 0.2 * step(0.5, fract(y * 6.0)));
                is_win = 0.0;
            }
        }
        if (is_win > 0.5) {
            glass = 1.0;
            albedo = vec3f(0.04, 0.05, 0.06);
            // 鐵窗
            if (kind == 0u && fl >= 1.0 && fract(seed * 0.0007 + bi * 0.37) > 0.5) {
                let bars = step(0.85, fract(bx * 8.0)) + step(0.9, fract(fy * 5.0));
                if (bars > 0.5) { albedo = vec3f(0.18, 0.18, 0.17); glass = 0.0; }
            }
            let lit = step(0.55 - night * 0.2, win_rand);
            emissive = mix(vec3f(1.0, 0.72, 0.42), vec3f(0.85, 0.9, 1.0), step(0.75, win_rand)) * lit * night * 2.2;
        } else if (fy < 0.08 && kind == 0u) {
            albedo *= 0.75;
        }
        // 冷氣外機
        if (kind == 0u && fl >= 1.0 && fy > 0.05 && fy < 0.22 && bx > 0.55 && bx < 0.85 && fract(win_rand * 7.3) > 0.5) {
            albedo = vec3f(0.78, 0.78, 0.75);
            glass = 0.0;
            emissive = vec3f(0.0);
        }
        // 招牌：正面一樓上方橫招牌 + 直立霓虹招牌
        if (front && kind == 0u) {
            let nc = neon_color(fract(seed * 0.00031));
            let band = step(floor_h * 0.82, y) * step(y, floor_h * 1.25);
            if (band > 0.5) {
                albedo = mix(vec3f(0.9), nc, 0.6);
                emissive = nc * (0.25 + night * 3.0);
                glass = 0.0;
            }
            let vs_x = sz.x * 0.5 - 1.1;
            let vertical = step(abs(lp.x - vs_x), 0.45) * step(floor_h * 1.4, y) * step(y, min(sz.y - 1.0, floor_h * 6.0));
            if (vertical > 0.5 && fract(seed * 0.0013) > 0.35) {
                let nc2 = neon_color(fract(seed * 0.00071));
                let chars = step(0.25, fract(y / 1.1)) * step(0.15, abs(lp.x - vs_x) * 2.0 + 0.1);
                albedo = mix(nc2 * 0.5, vec3f(0.95), chars);
                emissive = nc2 * (0.3 + night * 5.0) * (0.6 + 0.4 * chars);
                glass = 0.0;
            }
        }
        rough = 0.75;
    } else if (kind == 1u) {
        // 新城住宅：淺色外牆，陽台
        albedo = mix(vec3f(0.78, 0.76, 0.72), vec3f(0.70, 0.66, 0.60), fract(seed * 0.0001));
        let balcony = step(0.82, fy) * step(0.1, bx);
        if (is_win > 0.5) {
            glass = 1.0;
            albedo = vec3f(0.05, 0.07, 0.09);
            let lit = step(0.5 - night * 0.15, win_rand);
            emissive = mix(vec3f(1.0, 0.85, 0.65), vec3f(0.9, 0.95, 1.0), step(0.6, win_rand)) * lit * night * 1.8;
        } else if (balcony > 0.5) {
            albedo *= 0.8;
        }
        rough = 0.7;
    } else if (kind == 2u || kind == 9u) {
        // CBD 玻璃幕牆
        let tint = fract(seed * 0.00017);
        albedo = mix(vec3f(0.10, 0.16, 0.20), vec3f(0.14, 0.15, 0.16), step(0.5, tint));
        let mull = step(0.92, bx) + step(0.94, fy);
        glass = 1.0 - min(mull, 1.0);
        metal = 0.6;
        rough = 0.08;
        if (mull > 0.5) {
            albedo = vec3f(0.55, 0.57, 0.6);
            rough = 0.4;
        }
        let lit = step(0.45 - night * 0.1, win_rand) * glass;
        emissive = vec3f(0.95, 0.95, 1.0) * lit * night * 1.2;
        // LED 立面燈光秀
        let t = G.cam_pos.w;
        let show = 0.5 + 0.5 * sin(y * 0.05 - t * 1.5 + seed * 0.01);
        let ledc = mix(vec3f(0.1, 0.5, 1.0), vec3f(1.0, 0.2, 0.7), 0.5 + 0.5 * sin(t * 0.3 + seed));
        emissive += ledc * step(0.92, show) * night * 3.0 * step(0.94, fract(bx * 1.0 + 0.03));
        // 塔頂冠燈
        if (y > sz.y - 6.0) {
            emissive += vec3f(1.0, 0.95, 0.85) * night * 4.0 * select(1.0, 2.0, kind == 9u);
            albedo = vec3f(0.6);
        }
        if (kind == 9u && y > sz.y - 25.0 && fract(t * 0.5) < 0.1) {
            emissive += vec3f(1.0, 0.1, 0.1) * night * 6.0;
        }
    } else if (kind == 3u) {
        // 裙樓商舖
        albedo = vec3f(0.6, 0.6, 0.62);
        if (fy > 0.1 && fy < 0.85 && fl < 2.0) {
            glass = 1.0;
            albedo = vec3f(0.06, 0.07, 0.08);
            emissive = vec3f(1.0, 0.9, 0.75) * night * 2.0 * step(0.3, win_rand);
        }
    } else if (kind == 5u) {
        // 加油站：白色雨棚 + 色帶
        albedo = vec3f(0.85);
        if (y > sz.y - 1.2 && !roof) {
            albedo = vec3f(0.05, 0.55, 0.3);
            emissive = vec3f(0.1, 0.9, 0.5) * night * 2.0;
        }
        if (in.ln.y < -0.5) {
            emissive = vec3f(1.0, 0.98, 0.9) * (0.5 + night * 4.0);
        }
        if (y < sz.y - 1.3 && !roof && abs(lp.x) < sz.x * 0.5 - 0.5 && abs(lp.z) < sz.z * 0.5 - 0.5) {
            discard;
        }
        // 雨棚下方挖空，只留柱子
        if (y < sz.y - 1.3 && !roof) {
            let post = step(sz.x * 0.5 - 0.6, abs(lp.x)) * step(sz.z * 0.5 - 0.6, abs(lp.z));
            if (post < 0.5) { discard; }
        }
    } else if (kind == 7u) {
        // 燈塔
        albedo = select(vec3f(0.9), vec3f(0.75, 0.1, 0.08), fract(y / 5.0) > 0.5);
        if (y > sz.y - 3.0) {
            albedo = vec3f(0.1);
            emissive = vec3f(1.0, 0.95, 0.8) * (0.2 + night * 10.0) * (0.5 + 0.5 * sin(G.cam_pos.w * 2.0 + lp.x));
        }
    }
    if (roof) {
        albedo = vec3f(0.30, 0.30, 0.29) * (0.85 + 0.2 * vnoise(lp.xz * 0.5));
        if (kind == 0u && vnoise(lp.xz * 0.3 + seed) > 0.62) {
            albedo = vec3f(0.42, 0.45, 0.48); // 頂樓鐵皮
        }
        glass = 0.0;
        emissive = vec3f(0.0);
        rough = 0.9;
        metal = 0.0;
    }
    // 地面附近環境遮蔽
    let ao = mix(0.55, 1.0, smoothstep(0.0, 3.0, y));
    let sh = shadow_at(in.rel, n);
    var c: vec3f;
    if (glass > 0.5) {
        let v = -normalize(in.rel);
        let r = reflect(-v, n);
        let fr = fresnel(select(0.04, 0.12, metal > 0.0), dot(n, v));
        let refl = sky_radiance(normalize(vec3f(r.x, max(r.y, 0.03), r.z)));
        c = albedo * sky_ambient(n) * 0.5 + refl * mix(fr, 0.55, metal) + ggx_spec(n, v, G.light_dir.xyz, 0.05) * G.light_color.rgb * sh;
    } else {
        c = shade(albedo * ao, n, rough, metal, in.rel, sh);
    }
    c += emissive;
    c = apply_fog(c, in.rel);
    return vec4f(c, 1.0);
}

// ---------- 車輛 ----------
struct CIn {
    @location(0) local: vec3f,
    @location(1) normal: vec3f,
    @location(2) m0: vec4f,
    @location(3) m1: vec4f,
    @location(4) m2: vec4f,
    @location(5) m3: vec4f,
    @location(6) color: vec4f,   // rgb, a = 材質
};
struct COut {
    @builtin(position) pos: vec4f,
    @location(0) rel: vec3f,
    @location(1) n: vec3f,
    @location(2) color: vec4f,
    @location(3) lp: vec3f,
};

@vertex
fn vs_car(v: CIn) -> COut {
    let m = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    let rel = (m * vec4f(v.local, 1.0)).xyz;
    var o: COut;
    o.pos = G.view_proj * vec4f(rel, 1.0);
    o.rel = rel;
    o.n = normalize((m * vec4f(v.normal, 0.0)).xyz);
    o.color = v.color;
    o.lp = v.local;
    return o;
}

@vertex
fn vs_car_shadow(v: CIn) -> @builtin(position) vec4f {
    let m = mat4x4f(v.m0, v.m1, v.m2, v.m3);
    return G.shadow_mat * vec4f((m * vec4f(v.local, 1.0)).xyz, 1.0);
}

@fragment
fn fs_car(in: COut) -> @location(0) vec4f {
    let mat = i32(in.color.a + 0.5);
    let n = normalize(in.n);
    let v = -normalize(in.rel);
    let sh = shadow_at(in.rel, n);
    var c = vec3f(0.0);
    if (mat == 0) {
        // 車漆：底色 + 金屬片閃點 + 清漆
        let base = in.color.rgb;
        let flake = step(0.985, hash31(floor(in.lp * 900.0)));
        let l = G.light_dir.xyz;
        var col = shade(base, n, 0.35, 0.3, in.rel, sh);
        col += G.light_color.rgb * flake * pow(max(dot(reflect(-l, n), v), 0.0), 8.0) * sh * 0.8;
        let r = reflect(-v, n);
        let fr = fresnel(0.05, dot(n, v));
        col += sky_radiance(normalize(vec3f(r.x, max(r.y, 0.02), r.z))) * fr;
        col += ggx_spec(n, v, l, 0.03) * G.light_color.rgb * sh;
        c = col;
    } else if (mat == 1) {
        // 玻璃
        let r = reflect(-v, n);
        let fr = fresnel(0.04, dot(n, v));
        c = vec3f(0.01) + sky_radiance(normalize(vec3f(r.x, max(r.y, 0.02), r.z))) * mix(0.15, 1.0, fr) + ggx_spec(n, v, G.light_dir.xyz, 0.02) * G.light_color.rgb * sh;
    } else if (mat == 2) {
        // 輪胎
        c = shade(vec3f(0.03), n, 0.9, 0.0, in.rel, sh);
    } else if (mat == 3) {
        // 輪框
        c = shade(vec3f(0.7), n, 0.25, 1.0, in.rel, sh);
    } else if (mat == 4) {
        // 頭燈
        c = vec3f(1.0, 0.95, 0.85) * (0.6 + G.light_dir.w * 12.0);
    } else {
        // 尾燈（剎車時更亮，由顏色亮度傳入）
        c = vec3f(1.0, 0.05, 0.03) * (0.4 + G.light_dir.w * 3.0) * in.color.r;
    }
    c = apply_fog(c, in.rel);
    return vec4f(c, 1.0);
}
