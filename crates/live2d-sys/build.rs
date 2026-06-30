use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-env-changed=LIVE2D_CUBISM_SDK_DIR");
    if env::var_os("CARGO_FEATURE_CUBISM_CORE").is_none() {
        return;
    }

    let sdk_dir = env::var_os("LIVE2D_CUBISM_SDK_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
                .map(|dir| dir.join("vendor").join("live2d-cubism-sdk"))
        })
        .unwrap_or_else(|| {
            panic!("LIVE2D_CUBISM_SDK_DIR must point to the official Cubism SDK for Native when feature cubism-core is enabled")
        });
    if !sdk_dir.exists() {
        panic!(
            "LIVE2D_CUBISM_SDK_DIR does not exist: {}",
            sdk_dir.display()
        );
    }

    let target = env::var("TARGET").unwrap_or_default();
    let import_lib =
        find_best(&sdk_dir, &["lib"], "Live2DCubismCore", &target).unwrap_or_else(|| {
            panic!(
                "Live2DCubismCore import library was not found under {}",
                sdk_dir.display()
            )
        });
    let dll = find_best(&sdk_dir, &["dll"], "Live2DCubismCore", &target);

    println!(
        "cargo:rustc-link-search=native={}",
        import_lib.parent().unwrap().display()
    );
    let stem = import_lib.file_stem().unwrap().to_string_lossy();
    println!("cargo:rustc-link-lib=dylib={stem}");

    if let Some(dll) = dll {
        copy_runtime_dll(&dll);
    }
}

fn find_best(root: &Path, extensions: &[&str], name_prefix: &str, target: &str) -> Option<PathBuf> {
    let mut matches = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let matches_ext = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| extensions.iter().any(|ext| value.eq_ignore_ascii_case(ext)))
                .unwrap_or(false);
            let matches_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|value| value.starts_with(name_prefix))
                .unwrap_or(false);
            if matches_ext && matches_name {
                matches.push(path);
            }
        }
    }
    matches.sort_by_key(|path| selection_score(path, target));
    matches.into_iter().next()
}

fn selection_score(path: &Path, target: &str) -> i32 {
    let normalized = path.to_string_lossy().replace('\\', "/").to_lowercase();
    let mut score = 1000;
    if target.contains("x86_64") {
        if normalized.contains("/windows/x86_64/") {
            score -= 500;
        }
        if normalized.contains("/windows/x86/") {
            score += 500;
        }
    }
    if target.contains("i686") || target.contains("x86-pc") {
        if normalized.contains("/windows/x86/") {
            score -= 500;
        }
        if normalized.contains("/windows/x86_64/") {
            score += 500;
        }
    }
    if normalized.contains("/core/dll/") {
        score -= 100;
    }
    if normalized.contains("/143/") {
        score -= 30;
    } else if normalized.contains("/142/") {
        score -= 20;
    } else if normalized.contains("/141/") {
        score -= 10;
    }
    if normalized.ends_with("_md.lib") {
        score -= 4;
    } else if normalized.ends_with("_mt.lib") {
        score -= 3;
    } else if normalized.ends_with(".lib") {
        score -= 2;
    }
    score
}

fn copy_runtime_dll(dll: &Path) {
    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(profile_dir) = out_dir.ancestors().nth(3).map(Path::to_path_buf) else {
        return;
    };
    let target = profile_dir.join(dll.file_name().unwrap());
    let _ = fs::copy(dll, target);
}
