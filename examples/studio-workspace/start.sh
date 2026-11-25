cargo build -p nve-studio-workspace
cargo build -p nve-example-counter

./hosts/web-shell/build.sh --dev --help
./hosts/macos-shell/build/nve_macos_shell --dylib target/debug/libnve_studio_workspace.dylib
./hosts/gtk-shell/build/nve_gtk_shell --dylib target/debug/libnve_studio_workspace.dylib
