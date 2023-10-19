use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    let root = Path::new("../..").canonicalize()?;

    let dawn_src_dir = env::var("DAWN_SRC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("external/dawn"));

    let build_target = env::var("TARGET").unwrap();

    let dawn_build_dir = env::var(format!("DAWN_BUILD_DIR_{}", build_target.replace('-', "_")))
        .map(PathBuf::from)
        .or_else(|_| env::var("DAWN_BUILD_DIR").map(PathBuf::from))
        .unwrap_or_else(|_| root.join("build/dawn").join(&build_target));

    println!("cargo:rerun-if-env-changed=DAWN_SRC_DIR");
    println!("cargo:rerun-if-env-changed=DAWN_BUILD_DIR");

    let dawn_lib_dir = dawn_build_dir.join("lib");
    let dawn_gen_dir = dawn_build_dir.join("gen");

    println!("cargo:rustc-link-search=native={}", dawn_lib_dir.display());

    let common_libs = [
        "absl_base",
        "SPIRV-Tools-opt",
        "tint_lang_wgsl_helpers",
        "tint_utils_debug",
        "absl_int128",
        "tint_api",
        "tint_lang_wgsl_inspector",
        "tint_utils_diagnostic",
        "absl_log_severity",
        "tint_api_common",
        "tint_lang_wgsl_intrinsic",
        "tint_utils_generator",
        "absl_raw_logging_internal",
        "tint_api_options",
        "tint_lang_wgsl_program",
        "tint_utils_ice",
        "absl_spinlock_wait",
        "tint_lang_core",
        "tint_lang_wgsl_reader",
        "tint_utils_id",
        "absl_str_format_internal",
        "tint_lang_core_constant",
        "tint_lang_wgsl_reader_lower",
        "tint_utils_macros",
        "absl_strings",
        "tint_lang_core_intrinsic",
        "tint_lang_spirv_reader_common",
        "tint_lang_wgsl_reader_parser",
        "tint_utils_math",
        "absl_strings_internal",
        "tint_lang_core_ir",
        "tint_lang_wgsl_reader_program_to_ir",
        "tint_utils_memory",
        "absl_string_view",
        "tint_lang_wgsl_resolver",
        "tint_utils_reflection",
        "absl_throw_delegate",
        "tint_lang_core_type",
        "tint_lang_wgsl_sem",
        "tint_utils_result",
        "dawn_common",
        "tint_lang_hlsl_writer_common",
        "tint_lang_wgsl_writer",
        "tint_utils_rtti",
        "dawncpp_headers",
        "tint_lang_msl_writer",
        "tint_lang_wgsl_writer_ast_printer",
        "tint_utils_strconv",
        "dawn_headers",
        "tint_lang_msl_writer_ast_printer",
        "tint_lang_wgsl_writer_ast_printer",
        "tint_utils_symbol",
        "dawn_native",
        "tint_lang_msl_writer_ast_raise",
        "tint_lang_wgsl_writer_ir_to_program",
        "tint_utils_text",
        "dawn_platform",
        "tint_lang_msl_writer_common",
        "tint_lang_wgsl",
        "tint_lang_wgsl_writer",
        "tint_utils_traits",
        "dawn_proc",
        "tint_lang_msl_writer_printer",
        "tint_lang_wgsl_ast",
        "tint_lang_wgsl_writer_syntax_tree_printer",
        "SPIRV-Tools",
        "tint_lang_msl_writer_raise",
        "tint_lang_wgsl_ast_transform",
        "tint_utils_containers",
    ];

    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY")?;

    for lib in common_libs {
        let lib_name = if target_family == "windows" {
            format!("{lib}.lib")
        } else if target_family == "unix" {
            format!("lib{lib}.a")
        } else {
            panic!("unsupported target_family '{target_family}'");
        };

        let path = dawn_lib_dir.join(lib_name);

        if target_os == "linux"
            && !Command::new("ar")
                .arg("d")
                .arg(&path)
                .arg("Placeholder.cpp.o")
                .status()?
                .success()
        {
            panic!("ar command failed");
        }

        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-link-lib=static={lib}");
    }

    let platform_libs: &[_] = match target_os.as_str() {
        "linux" => &[
            "tint_lang_wgsl_ir",
            "tint_lang_wgsl_writer_raise",
            "tint_lang_spirv",
            "tint_lang_spirv_intrinsic",
            "tint_lang_spirv_ir",
            "tint_lang_spirv_reader",
            "tint_lang_spirv_reader_ast_lower",
            "tint_lang_spirv_reader_ast_parser",
            "tint_lang_spirv_type",
            "tint_lang_core_ir_transform",
            "tint_lang_spirv_writer",
            "tint_lang_spirv_writer_ast_printer",
            "tint_lang_spirv_writer_ast_raise",
            "tint_lang_spirv_writer_common",
            "tint_lang_spirv_writer_printer",
            "tint_lang_spirv_writer_raise",
        ],
        _ => &[], // add a case here if it's not working
    };

    for lib in platform_libs {
        let lib_name = if target_family == "windows" {
            format!("{lib}.lib")
        } else if target_family == "unix" {
            format!("lib{lib}.a")
        } else {
            panic!("unsupported target_family '{target_family}'");
        };

        let path = dawn_lib_dir.join(lib_name);

        if target_os == "linux"
            && !Command::new("ar")
                .arg("d")
                .arg(&path)
                .arg("Placeholder.cpp.o")
                .status()?
                .success()
        {
            panic!("ar command failed");
        }

        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-link-lib=static={lib}");
    }

    // Additional platform-specific libraries we need to link
    let libs: &[_] = match target_os.as_str() {
        "windows" => &["dxguid"],
        "macos" => &[
            "framework=Foundation",
            "framework=IOKit",
            "framework=IOSurface",
        ],
        "linux" => &["X11"],
        _ => &[],
    };

    for lib in libs {
        println!("cargo:rustc-link-lib={lib}");
    }

    let out = PathBuf::from(env::var("OUT_DIR")?);

    let webgpu_header = dawn_gen_dir.join("include/dawn/webgpu.h");

    // Generate bindings for the webgpu header.
    bindgen::builder()
        .header(webgpu_header.to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .layout_tests(false)
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(out.join("webgpu.rs"))
        .expect("failed to write bindings to output file");

    let mut cc = cc::Build::new();

    cc.file("src/lib.cpp")
        .cpp(true)
        .include(dawn_src_dir.join("include"))
        .include(dawn_gen_dir.join("include"));

    if build_target == "x86_64-pc-windows-msvc" {
        cc.flag("/std:c++17").flag("/MD");
    } else {
        cc.flag("-std=c++17");
    }

    // Compile and link the c++ wrapper code for dawn initialisation.
    cc.compile("dawn_init");

    println!("cargo:rerun-if-changed=src/lib.cpp");

    Ok(())
}
