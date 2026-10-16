fn main() {
    println!("cargo:rerun-if-changed=shaders");
    for f in [
        "shaders/glass_vs.hlsl",
        "shaders/blur_vs.hlsl",
        "shaders/glass_ps.hlsl",
        "shaders/blur_down.hlsl",
        "shaders/blur_up.hlsl",
        "shaders/env_ema.hlsl",
    ] {
        println!("cargo:rerun-if-changed={f}");
    }
}
