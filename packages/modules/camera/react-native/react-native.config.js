"use strict";

const path = require("path");

/**
 * React Native autolinking manifest for `@nve/camera` (optional VisionCamera v5 adapter
 * and `CameraPreview` binary codec).
 */
module.exports = {
  dependency: {
    platforms: {
      ios: {
        podspecPath: path.join(__dirname, "NveCamera.podspec"),
      },
      android: {
        sourceDir: path.resolve(__dirname, "../android"),
        packageImportPath: "import dev.nve.camera.NveCameraBackend;",
        libraryName: "NveCamera",
        componentDescriptors: ["NveCameraPreviewComponentDescriptor"],
        cmakeListsPath: null,
      },
    },
  },
};
