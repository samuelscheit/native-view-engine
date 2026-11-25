use std::sync::Arc;

use super::*;

#[test]
fn typed_native_props_and_commands_roundtrip_zero_serialization_and_match_wire() {
    let blur = BlurSurfaceProps {
        material: BlurMaterial::Thick,
        intensity: 0.75,
        tint: Some(0x1122_3380),
    };
    let typed_props = blur.to_native_props().unwrap();
    assert_eq!(BlurSurfaceProps::from_native_props(&typed_props).unwrap(), blur);
    // Wire bytes produced from NativeProps match direct NVNP encoding byte-for-byte.
    assert_eq!(typed_props.to_nvnp_bytes().unwrap(), blur.to_prop_bytes().unwrap());
    assert_eq!(
        NativeProps::from_nvnp_bytes(&blur.to_prop_bytes().unwrap()).unwrap(),
        typed_props
    );

    let carrier = TypedNativeViewProps::with_typed(
        BlurSurface::KIND,
        typed_props.clone(),
        Arc::new(blur),
    );
    assert_eq!(carrier.downcast_ref::<BlurSurfaceProps>(), Some(&blur));

    let nav = WebViewCommand::Navigate("https://nve.dev/docs".into());
    let typed_cmd = nav.to_native_command().unwrap();
    assert_eq!(typed_cmd.command_name(), "navigate");
    assert_eq!(WebViewCommand::from_native_command(&typed_cmd).unwrap(), nav);
    assert_eq!(typed_cmd.to_nvnc_bytes().unwrap(), nav.encode_command().unwrap());

    let seek = VideoPlayerCommand::SeekMs(42_000);
    let typed_seek = seek.to_native_command().unwrap();
    assert_eq!(VideoPlayerCommand::from_native_command(&typed_seek).unwrap(), seek);
    assert_eq!(typed_seek.to_nvnc_bytes().unwrap(), seek.encode_command().unwrap());

    let we = WebElementProps::new("svg").with_attr("viewBox", "0 0 24 24");
    let we_props = we.to_native_props().unwrap();
    let we_dec = DecodedWebElementProps::from_native_props(&we_props).unwrap();
    assert_eq!(we_dec, we.to_decoded());
}
