//! WGSL 原始碼。場景著色器在前面拼接 common.wgsl。

pub const COMMON: &str = include_str!("shaders/common.wgsl");
pub const SKY: &str = include_str!("shaders/sky.wgsl");
pub const TERRAIN: &str = include_str!("shaders/terrain.wgsl");
pub const OBJECTS: &str = include_str!("shaders/objects.wgsl");
pub const POST: &str = include_str!("shaders/post.wgsl");

pub fn terrain() -> String {
    format!("{}\n{}", COMMON, TERRAIN)
}

pub fn objects() -> String {
    format!("{}\n{}", COMMON, OBJECTS)
}

pub fn all() -> Vec<(&'static str, String)> {
    vec![
        ("sky", SKY.to_string()),
        ("terrain", terrain()),
        ("objects", objects()),
        ("post", POST.to_string()),
    ]
}
