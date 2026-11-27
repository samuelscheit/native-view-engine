#![allow(dead_code)]

use nve_codegen::{
    CommandSpec, DefaultValue, EventBubbling, EventSpec, ExportBackend, NveExportSpec, ObjectField,
    ParamSpec, PropSpec, PropType,
};

pub const EMBEDDED_SWITCH_SPEC: &str = r#"
/**
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 * @flow strict-local
 */
import type {ViewProps} from '../../../../../Libraries/Components/View/ViewPropTypes';
import type {ColorValue} from '../../../../../Libraries/StyleSheet/StyleSheet';
import type {BubblingEventHandler, Int32, WithDefault} from '../../../../../Libraries/Types/CodegenTypes';
import codegenNativeCommands from '../../../../../Libraries/Utilities/codegenNativeCommands';
import codegenNativeComponent from '../../../../../Libraries/Utilities/codegenNativeComponent';

type NativeSwitchChangeEvent = Readonly<{
  value: boolean,
  target: Int32,
}>;

type SwitchNativeProps = Readonly<{
  ...ViewProps,
  disabled?: WithDefault<boolean, false>,
  value?: WithDefault<boolean, false>,
  tintColor?: ?ColorValue,
  onTintColor?: ?ColorValue,
  thumbTintColor?: ?ColorValue,
  onChange?: ?BubblingEventHandler<NativeSwitchChangeEvent>,
}>;

interface NativeCommands {
  readonly setValue: (viewRef: React.ElementRef<ComponentType>, value: boolean) => void;
}

export const Commands: NativeCommands = codegenNativeCommands<NativeCommands>({
  supportedCommands: ['setValue'],
});

export default codegenNativeComponent<SwitchNativeProps>('Switch', {
  paperComponentName: 'RCTSwitch',
  excludedPlatforms: ['android'],
  interfaceOnly: true,
}) as ComponentType;
"#;

pub fn assert_balanced_delimiters(label: &str, code: &str) {
    let mut braces = 0i32;
    let mut parens = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == '*' && next == Some('/') {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if in_single {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }

        if c == '/' && next == Some('/') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if c == '/' && next == Some('*') {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if c == '\'' && !label.ends_with(".rs") {
            in_single = true;
            i += 1;
            continue;
        }
        if c == '"' {
            in_double = true;
            i += 1;
            continue;
        }

        match c {
            '{' => braces += 1,
            '}' => braces -= 1,
            '(' => parens += 1,
            ')' => parens -= 1,
            _ => {}
        }
        assert!(braces >= 0, "{label}: negative brace balance near char {i}");
        assert!(parens >= 0, "{label}: negative paren balance near char {i}");
        i += 1;
    }

    assert_eq!(braces, 0, "{label}: unbalanced curly braces");
    assert_eq!(parens, 0, "{label}: unbalanced parentheses");
}

pub fn counter_card_export_spec(backend: ExportBackend) -> NveExportSpec {
    NveExportSpec {
        name: "NveCounterCard".to_owned(),
        backend,
        rust_vtable_symbol: Some("nve_counter_card_vtable".to_owned()),
        android_package: Some("dev.nve.components".to_owned()),
        doc: Some("Interactive counter card rendered natively by NVE.".to_owned()),
        props: vec![
            PropSpec {
                name: "initialCount".to_owned(),
                prop_type: PropType::Int32,
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::Int(0)),
                doc: Some("Initial counter value.".to_owned()),
            },
            PropSpec {
                name: "accentColor".to_owned(),
                prop_type: PropType::Color,
                optional: true,
                nullable: true,
                default_value: None,
                doc: Some("Optional accent tint color.".to_owned()),
            },
            PropSpec {
                name: "variant".to_owned(),
                prop_type: PropType::StringEnum {
                    variants: vec!["solid".into(), "outline".into(), "ghost".into()],
                },
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::String("solid".into())),
                doc: Some("Visual button style variant.".to_owned()),
            },
            PropSpec {
                name: "stepScale".to_owned(),
                prop_type: PropType::Float,
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::Float(1.0)),
                doc: None,
            },
            PropSpec {
                name: "disabled".to_owned(),
                prop_type: PropType::Bool,
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::Bool(false)),
                doc: None,
            },
        ],
        events: vec![
            EventSpec {
                name: "onCountChange".to_owned(),
                bubbling: EventBubbling::Direct,
                optional: true,
                paper_top_level_name: None,
                payload: vec![
                    ObjectField {
                        name: "count".to_owned(),
                        field_type: PropType::Int32,
                        optional: false,
                        nullable: false,
                    },
                    ObjectField {
                        name: "delta".to_owned(),
                        field_type: PropType::Int32,
                        optional: false,
                        nullable: false,
                    },
                ],
                payload_type_name: Some("CountChangeEvent".to_owned()),
                doc: Some("Emitted whenever the counter value changes.".to_owned()),
            },
            EventSpec {
                name: "onReset".to_owned(),
                bubbling: EventBubbling::Bubbling,
                optional: true,
                paper_top_level_name: None,
                payload: Vec::new(),
                payload_type_name: None,
                doc: None,
            },
        ],
        commands: vec![CommandSpec {
            name: "setCount".to_owned(),
            params: vec![ParamSpec {
                name: "value".to_owned(),
                param_type: PropType::Int32,
                optional: false,
            }],
        }],
    }
}
