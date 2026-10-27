// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "MediaLayer",
  platforms: [
    .iOS(.v17)
  ],
  products: [
    .library(name: "MediaLayer", targets: ["MediaLayer"]),
    .library(name: "MediaLayerUI", targets: ["MediaLayerUI"]),
    .library(name: "MediaLayerTesting", targets: ["MediaLayerTesting"])
  ],
  dependencies: [
    .package(url: "https://github.com/kean/Nuke", from: "13.2.0")
  ],
  targets: [
    .binaryTarget(name: "medialayerFFI", path: "MediaLayerFFI.xcframework"),
    .target(
      name: "MediaLayer",
      dependencies: ["medialayerFFI"],
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .target(
      name: "MediaLayerUI",
      dependencies: [
        "MediaLayer",
        .product(name: "Nuke", package: "Nuke")
      ],
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .target(
      name: "MediaLayerTesting",
      dependencies: ["MediaLayer"],
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .testTarget(
      name: "MediaLayerTests",
      dependencies: ["MediaLayer", "MediaLayerTesting"]
    ),
    .testTarget(
      name: "MediaLayerUITests",
      dependencies: ["MediaLayerUI", "MediaLayer", "MediaLayerTesting"],
      resources: [.copy("Fixtures")],
      swiftSettings: [.swiftLanguageMode(.v5)]
    )
  ]
)
