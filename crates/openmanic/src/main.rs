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
    if let Err(error) = openmanic::composition::run_process() {
        eprintln!("OpenManic could not start: {}", error.safe_summary());
        std::process::exit(1);
    }
}
