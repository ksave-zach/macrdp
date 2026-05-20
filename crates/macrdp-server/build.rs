use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Candidate paths for Swift Concurrency runtime library
fn swift_lib_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // 1. User-specified path (highest priority)
    if let Ok(path) = env::var("SWIFT_LIB_PATH") {
        candidates.push(PathBuf::from(path));
    }

    // 2. System Swift runtime (usually sufficient on macOS 12.3+)
    candidates.push(PathBuf::from("/usr/lib/swift"));

    // 3. Detect via xcode-select
    if let Ok(output) = Command::new("xcode-select").arg("-p").output() {
        if output.status.success() {
            let dev_path = String::from_utf8_lossy(&output.stdout).trim().to_string();

            if dev_path.contains("CommandLineTools") {
                // Command Line Tools installation
                candidates.push(PathBuf::from(format!(
                    "{dev_path}/usr/lib/swift-5.5/macosx"
                )));
                candidates.push(PathBuf::from(format!(
                    "{dev_path}/usr/lib/swift/macosx"
                )));
            } else {
                // Full Xcode installation
                candidates.push(PathBuf::from(format!(
                    "{dev_path}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift-5.5/macosx"
                )));
                candidates.push(PathBuf::from(format!(
                    "{dev_path}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx"
                )));
            }
        }
    }

    // 4. Common fallback paths
    candidates.push(PathBuf::from(
        "/Library/Developer/CommandLineTools/usr/lib/swift-5.5/macosx",
    ));
    candidates.push(PathBuf::from(
        "/Library/Developer/CommandLineTools/usr/lib/swift/macosx",
    ));

    candidates
}

fn main() {
    let target_lib = "libswift_Concurrency.dylib";

    // Check user-specified path first
    if let Ok(path) = env::var("SWIFT_LIB_PATH") {
        let p = PathBuf::from(&path);
        if p.join(target_lib).exists() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", p.display());
            return;
        }
    }

    // Search candidates — if we find the dylib on disk, add its directory as rpath
    for candidate in swift_lib_candidates() {
        if candidate.join(target_lib).exists() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", candidate.display());
            return;
        }
    }

    // Fallback: the dylib may live only in the dyld shared cache (macOS 26+).
    // The Swift compiler emits @rpath/libswift_Concurrency.dylib references,
    // so the binary must have an rpath to resolve it. /usr/lib/swift is the
    // canonical location and is always served by the cache even when no file
    // exists on disk.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
