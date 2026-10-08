// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "MediaLayer",
  platforms: [
    .iOS(.v17),
    .macOS(.v14)
  ],
  products: [
    .library(name: "MediaLayer", targets: ["MediaLayer"])
  ],
  targets: [
    .binaryTarget(name: "medialayerFFI", path: "MediaLayerFFI.xcframework"),
    .target(
      name: "MediaLayer",
      dependencies: ["medialayerFFI"],
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .testTarget(
      name: "MediaLayerTests",
      dependencies: ["MediaLayer"]
    )
  ]
)
