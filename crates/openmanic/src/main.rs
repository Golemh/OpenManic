//! OpenManic executable entry point.

#![forbid(unsafe_code)]

#[cfg(all(feature = "renderer-wgpu", feature = "renderer-glow"))]
compile_error!("select exactly one renderer: renderer-wgpu or renderer-glow");

#[cfg(not(any(feature = "renderer-wgpu", feature = "renderer-glow")))]
compile_error!("select one renderer: renderer-wgpu or renderer-glow");

#[cfg(all(feature = "platform-windows", feature = "platform-linux"))]
compile_error!("select exactly one platform family: platform-windows or platform-linux");

#[cfg(not(any(feature = "platform-windows", feature = "platform-linux")))]
compile_error!("select one platform family: platform-windows or platform-linux");

fn main() {
    // Composition is intentionally introduced in small, independently testable steps. OM-295
    // provides the bootstrap foundations; the vertical-slice composition owns launching services
    // and the eframe shell after all adapter and storage wiring is available.
}
