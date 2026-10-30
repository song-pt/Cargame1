//! 在沒有 GPU 的環境裡用 naga 校驗所有 WGSL。

fn check(name: &str, src: &str) {
    let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}:\n{}", name, e.emit_to_string(src)));
    let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty());
    v.validate(&module).unwrap_or_else(|e| panic!("{}:\n{}", name, e.emit_to_string(src)));
}

#[test]
fn shaders_validate() {
    for (name, src) in game::shaders::all() {
        check(name, &src);
    }
}
