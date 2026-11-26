use std::sync::Arc;

use nve_filesystem::{
    decode_file_picker_options, decode_pick_files_response, decode_save_file_options,
    decode_save_file_response, decode_standard_path_request, decode_standard_path_response,
    encode_file_picker_options, FileFilter, FilePickerOptions, FilesystemClient, MemoryFilesystem,
    NativeFilesystem, SaveFileOptions, StandardPathKind,
};
use nve_host_services::{HostServiceRegistry, MemoryHostServices, ServiceError};

#[test]
fn filesystem_memory_native_and_truncation_fuzz() {
    let fs_mem = Arc::new(MemoryFilesystem::new());
    let host = MemoryHostServices::new().with_handler_arc(fs_mem.clone());
    let client = FilesystemClient::new(&host);

    for (kind, expected) in [
        (StandardPathKind::Data, "/mem/data"),
        (StandardPathKind::Cache, "/mem/cache"),
        (StandardPathKind::Config, "/mem/config"),
        (StandardPathKind::Temp, "/mem/tmp"),
        (StandardPathKind::Documents, "/mem/documents"),
        (StandardPathKind::Downloads, "/mem/downloads"),
    ] {
        assert_eq!(client.standard_path(kind).unwrap(), expected);
    }

    let pick_opts = FilePickerOptions {
        title: Some("Select Assets".into()),
        initial_dir: Some("/mem/documents".into()),
        filters: vec![FileFilter::new("Images", ["png", "webp"])],
        allow_multiple: true,
        pick_directories: false,
    };
    assert_eq!(
        client.pick_files(&pick_opts).unwrap(),
        vec!["/mem/documents/sample.txt".to_string()]
    );
    fs_mem.set_pick_files_result(Err(ServiceError::Cancelled));
    assert_eq!(client.pick_files(&pick_opts), Err(ServiceError::Cancelled));

    let save_opts = SaveFileOptions {
        title: Some("Export Scene".into()),
        default_name: Some("scene.nve".into()),
        initial_dir: None,
        filters: vec![FileFilter::new("NVE Binary", ["nve"])],
    };
    assert_eq!(
        client.save_file(&save_opts).unwrap().as_deref(),
        Some("/mem/documents/output.bin")
    );

    // Safe non-modal native standard paths.
    let native = HostServiceRegistry::new().with_handler(NativeFilesystem::new());
    let native_client = FilesystemClient::new(&native);
    for kind in [
        StandardPathKind::Data,
        StandardPathKind::Cache,
        StandardPathKind::Config,
        StandardPathKind::Temp,
        StandardPathKind::Documents,
        StandardPathKind::Downloads,
    ] {
        assert!(!native_client.standard_path(kind).unwrap().is_empty());
    }

    // Prefix truncation + PRNG fuzz.
    let valid_picker = encode_file_picker_options(&pick_opts).unwrap();
    for len in 0..valid_picker.len() {
        assert!(decode_file_picker_options(&valid_picker[..len]).is_err());
    }
    let mut seed: u64 = 0xF11E_5757_5EED_0002;
    let mut next_u8 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed & 0xFF) as u8
    };
    for i in 0..512 {
        let len = i % 64;
        let buf: Vec<u8> = (0..len).map(|_| next_u8()).collect();
        let _ = decode_standard_path_request(&buf);
        let _ = decode_standard_path_response(&buf);
        let _ = decode_file_picker_options(&buf);
        let _ = decode_pick_files_response(&buf);
        let _ = decode_save_file_options(&buf);
        let _ = decode_save_file_response(&buf);
    }
}
