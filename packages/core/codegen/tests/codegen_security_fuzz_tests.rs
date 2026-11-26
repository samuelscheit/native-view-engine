mod common;

use std::path::Path;

use common::specs::{assert_balanced_delimiters, EMBEDDED_SWITCH_SPEC};
use nve_codegen::{
    export_compose, export_flutter, export_rn_spec, export_swiftui, export_web_component,
    import_rn_source, parse_rn_spec, DefaultValue, EventBubbling, EventSpec, ExportBackend,
    NveExportSpec, ObjectField, PropSpec, PropType,
};

#[test]
fn naming_escapers_neutralize_interpolation_quotes_and_comment_breakout() {
    use nve_codegen::naming::{
        escape_c_like_string, escape_dart_single_quoted_string, escape_kotlin_string,
        escape_ts_single_quoted, format_float_literal, sanitize_block_comment,
        sanitize_single_line_comment,
    };

    assert_eq!(
        escape_kotlin_string("$HOME/${expr}\"\\a\n\r\t\0"),
        "\\$HOME/\\${expr}\\\"\\\\a\\n\\r\\t\\u0000"
    );
    assert_eq!(
        escape_dart_single_quoted_string("$HOME/${expr}'\\a\n\r\t\0"),
        "\\$HOME/\\${expr}\\'\\\\a\\n\\r\\t\\u0000"
    );
    assert_eq!(
        escape_c_like_string("\\(swift)\"\\a\n\r\t\0"),
        "\\\\(swift)\\\"\\\\a\\n\\r\\t\\0"
    );
    assert_eq!(
        escape_ts_single_quoted("it's\\a\n\r\t\0\u{2028}\u{2029}"),
        "it\\'s\\\\a\\n\\r\\t\\u0000\\u2028\\u2029"
    );

    let sanitized_line = sanitize_single_line_comment("tag\r\nevil();\u{2028}more\u{2029}end");
    assert!(!sanitized_line.contains('\n'));
    assert!(!sanitized_line.contains('\r'));
    assert!(!sanitized_line.contains('\u{2028}'));
    assert!(!sanitized_line.contains('\u{2029}'));

    let sanitized_block = sanitize_block_comment("hello */ evil();\r\n/* ok");
    assert!(!sanitized_block.contains("*/"));
    assert!(!sanitized_block.contains('\r'));

    assert_eq!(format_float_literal(2.0), "2.0");
    assert_eq!(format_float_literal(f64::NAN), "0.0");
    assert_eq!(format_float_literal(f64::INFINITY), "0.0");
}

#[test]
fn export_and_import_neutralize_string_interpolation_and_comment_breakout() {
    let mut spec = NveExportSpec::new("AdversarialWidget", ExportBackend::Skia);
    spec.doc = Some("Top doc */ evil();\nsecond line\r\nthird".to_owned());
    spec.props = vec![
        PropSpec {
            name: "templateText".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::String(
                "$HOME/${System.exit(1)} it's \\ \"quoted\" \\(swift)\nnext\r\tline".to_owned(),
            )),
            doc: Some("Prop doc line1\n*/ evil();\n// breakout".to_owned()),
        },
        PropSpec {
            name: "mode".to_owned(),
            prop_type: PropType::StringEnum {
                variants: vec![
                    "$HOME".to_owned(),
                    "${expr}".to_owned(),
                    "it's\\quoted\"mode\nbreak".to_owned(),
                ],
            },
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::String("$HOME".to_owned())),
            doc: None,
        },
    ];
    spec.events = vec![EventSpec {
        name: "onPayload".to_owned(),
        bubbling: EventBubbling::Direct,
        optional: true,
        paper_top_level_name: Some("top$HOME'\"\\".to_owned()),
        payload: vec![ObjectField {
            name: "detail".to_owned(),
            field_type: PropType::String,
            optional: false,
            nullable: false,
        }],
        payload_type_name: None,
        doc: Some("Event doc */ breakout\nnext".to_owned()),
    }];

    let compose = export_compose(&spec).expect("export_compose");
    let kt = &compose.compose_kt.content;
    assert!(kt.contains("\\$HOME/\\${System.exit(1)}"));
    assert!(!kt.replace("\\$", "").contains("$HOME"));
    assert!(!kt.replace("\\$", "").contains("${System.exit(1)}"));
    assert!(!kt.contains("*/ evil();"));
    assert_balanced_delimiters(&compose.compose_kt.relative_path, kt);

    let rn = export_rn_spec(&spec).expect("export_rn_spec");
    let rn_kt = &rn.android_view_manager_kt.content;
    assert!(rn_kt.contains("top\\$HOME"));
    assert!(!rn_kt.replace("\\$", "").contains("top$HOME"));
    assert!(!rn.native_component_ts.content.contains("*/ evil();"));
    assert_balanced_delimiters(&rn.native_component_ts.relative_path, &rn.native_component_ts.content);
    assert_balanced_delimiters(&rn.cpp_bridge_hpp.relative_path, &rn.cpp_bridge_hpp.content);
    assert_balanced_delimiters(&rn.cpp_bridge_cpp.relative_path, &rn.cpp_bridge_cpp.content);
    assert_balanced_delimiters(&rn.ios_component_view_mm.relative_path, &rn.ios_component_view_mm.content);
    assert_balanced_delimiters(&rn.android_view_manager_kt.relative_path, rn_kt);
    assert_balanced_delimiters(&rn.windows_component_view_hpp.relative_path, &rn.windows_component_view_hpp.content);

    let flutter = export_flutter(&spec).expect("export_flutter");
    let dart = &flutter.dart_widget.content;
    assert!(dart.contains("\\$HOME/\\${System.exit(1)} it\\'s \\\\ \"quoted\""));
    assert!(!dart.replace("\\$", "").contains("'$HOME"));
    assert!(!dart.replace("\\$", "").contains("${System.exit(1)}"));
    for line in dart.lines() {
        assert!(!line.trim_start().starts_with("*/ evil();"), "comment breakout in Dart output: {line}");
    }
    assert_balanced_delimiters(&flutter.dart_widget.relative_path, dart);

    let swiftui = export_swiftui(&spec).expect("export_swiftui");
    let swift = &swiftui.swift_view.content;
    assert!(swift.contains("\\\\(swift)"));
    for line in swift.lines() {
        assert!(!line.trim_start().starts_with("*/ evil();"), "comment breakout in Swift output: {line}");
    }
    assert_balanced_delimiters(&swiftui.swift_view.relative_path, swift);

    let web = export_web_component(&spec).expect("export_web_component");
    assert!(web.element_js.content.contains("it\\'s"));
    assert!(web.element_js.content.contains("\\nnext\\r\\tline"));
    assert_balanced_delimiters(&web.element_js.relative_path, &web.element_js.content);
    assert_balanced_delimiters(&web.types_dts.relative_path, &web.types_dts.content);

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let include_dir = manifest_dir.join("../../../cxx/nve-fabric-bridge/include");
    if include_dir.join("nve_rn_component.hpp").exists()
        && std::process::Command::new("clang++")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    {
        let tmp = std::env::temp_dir().join(format!(
            "nve_codegen_adversarial_cpp_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        common::write_generated_c_abi_headers(&tmp);
        rn.write_to_dir(&tmp).unwrap();
        let out = std::process::Command::new("clang++")
            .args(["-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
            .arg("-I").arg(&tmp)
            .arg("-I").arg(&include_dir)
            .arg(tmp.join(&rn.cpp_bridge_cpp.relative_path))
            .output()
            .expect("clang++ adversarial export_rn");
        assert!(
            out.status.success(),
            "adversarial export_rn C++20 failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    let adversarial_rn_spec = r#"
    import type { ViewProps } from 'react-native';
    import type { WithDefault } from 'react-native/Libraries/Types/CodegenTypes';
    import codegenNativeComponent from 'react-native/Libraries/Utilities/codegenNativeComponent';

    export interface NativeProps extends ViewProps {
      label?: WithDefault<string, '$HOME/${x} it\'s "quoted"\nline2'>;
    }

    export default codegenNativeComponent<NativeProps>('EvilTag\nfn injected() {} //');
    "#;
    let imported = import_rn_source(adversarial_rn_spec).expect("import adversarial RN spec");
    for line in imported.rust_module.content.lines() {
        assert!(!line.trim_start().starts_with("fn injected()"), "comment breakout in import_rn Rust output: {line}");
    }
    for line in imported.cpp_adapter.content.lines() {
        assert!(!line.trim_start().starts_with("fn injected()"), "comment breakout in import_rn C++ output: {line}");
    }
    assert_balanced_delimiters(&imported.rust_module.relative_path, &imported.rust_module.content);
    assert_balanced_delimiters(&imported.cpp_adapter.relative_path, &imported.cpp_adapter.content);
}

#[test]
fn fuzz_rn_lexer_and_parser_never_panic_on_adversarial_and_truncated_inputs() {
    use nve_codegen::frontend::rn_spec::lexer::tokenize;

    for (byte_idx, _) in EMBEDDED_SWITCH_SPEC.char_indices() {
        let prefix = &EMBEDDED_SWITCH_SPEC[..byte_idx];
        let _ = tokenize(prefix);
        let _ = parse_rn_spec(prefix);
    }

    let deep_cases = [
        format!("interface P {{ a: {}string{}; }} export default codegenNativeComponent<P>('X');", "(".repeat(256), ")".repeat(256)),
        format!("interface P {{ a: {}string; }} export default codegenNativeComponent<P>('X');", "?".repeat(256)),
        format!("interface P {{ a: {}string{}; }} export default codegenNativeComponent<P>('X');", "Readonly<".repeat(256), ">".repeat(256)),
        format!("interface P {{ {}x: string{} }} export default codegenNativeComponent<P>('X');", "a: { ".repeat(200), " }".repeat(200)),
        "/* unterminated block".to_owned(),
        "'unterminated string\\".to_owned(),
        "\"unterminated".to_owned(),
        "`unterminated".to_owned(),
        "// comment without newline".to_owned(),
        "interface A extends A { a: A } export default codegenNativeComponent<A>('A');".to_owned(),
        "type A = A; interface P { x: A } export default codegenNativeComponent<P>('A');".to_owned(),
    ];
    for case in &deep_cases {
        let _ = tokenize(case);
        let _ = parse_rn_spec(case);
    }

    let alphabet: &[&str] = &[
        "export", "default", "interface", "type", "const", "extends", "readonly",
        "codegenNativeComponent", "codegenNativeCommands", "TurboModuleRegistry",
        "getEnforcing", "get", "WithDefault", "DirectEventHandler", "BubblingEventHandler",
        "Readonly", "ViewProps", "string", "boolean", "Int32", "Float", "Double",
        "null", "undefined", "void", "true", "false", "{", "}", "{|", "|}", "(", ")",
        "[", "]", "<", ">", ":", ";", ",", ".", "...", "=>", "?", "|", "&", "=", "-", "+",
        "'str'", "\"$HOME/${x}\"", "'unterm", "/*", "*/", "//\n", "123", "-99999999999999999999",
        "0xZZ", " ", "\n", "\t", "🦀",
    ];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_u64 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for i in 0..1200 {
        let len = (next_u64() as usize % 36) + 1;
        let mut s = String::with_capacity(len * 8);
        for _ in 0..len {
            let tok = alphabet[(next_u64() as usize) % alphabet.len()];
            s.push_str(tok);
            if (next_u64() & 1) == 0 {
                s.push(' ');
            }
        }
        let _ = tokenize(&s);
        let _ = parse_rn_spec(&s);
        if i % 4 == 0 && !s.is_empty() {
            let cut = s.char_indices().nth((next_u64() as usize) % s.chars().count()).map(|(b, _)| b).unwrap_or(0);
            let _ = parse_rn_spec(&s[..cut]);
        }
    }
}
