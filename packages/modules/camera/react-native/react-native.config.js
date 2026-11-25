"use strict";

const path = require("path");

/**
 * React Native / React Native Windows (0.88) / React Native macOS autolinking manifest
 * for `@nve/camera`. Points directly at the shared native sources in `../android`,
 * `../apple` (via `./NveCamera.podspec`), and `./windows/CMakeLists.txt` with zero
 * duplicated camera code.
 */
module.exports = {
  dependency: {
    platforms: {
      ios: {
        podspecPath: path.join(__dirname, "NveCamera.podspec"),
      },
      macos: {
        podspecPath: path.join(__dirname, "NveCamera.podspec"),
      },
      android: {
        sourceDir: path.resolve(__dirname, "../android"),
        packageImportPath: "import dev.nve.camera.NveCameraBackend;",
        libraryName: "NveCamera",
        componentDescriptors: ["NveCameraPreviewComponentDescriptor"],
        cmakeListsPath: null,
      },
      windows: {
        sourceDir: path.join(__dirname, "windows"),
        cmakeListsPath: path.join(__dirname, "windows", "CMakeLists.txt"),
        solutionFile: null,
        projects: [
          {
            projectFile: "CMakeLists.txt",
            directDependency: true,
          },
        ],
      },
    },
  },
};
