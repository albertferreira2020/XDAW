//! Generates the demo session: `xdaw-make-demo <directory>`.

fn main() {
    let dir = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| "demo/XDAW Demo".into());
    let project = match xdaw_engine::demo::build_demo(&dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("demo generation failed: {e}");
            std::process::exit(1);
        }
    };
    let path = dir.join("XDAW Demo.xdaw");
    if let Err(e) = xdaw_project::io::save(&project, &path) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    println!("wrote {}", path.display());
}
