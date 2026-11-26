// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "MediaLayer",
  platforms: [.iOS(.v17), .macOS(.v14)],
  products: [
    .library(name: "MediaLayer", targets: ["MediaLayer"]),
    .library(name: "MediaLayerUI", targets: ["MediaLayerUI"]),
    .library(name: "MediaLayerTesting", targets: ["MediaLayerTesting"])
  ],
  dependencies: [
    .package(url: "https://github.com/kean/Nuke", from: "13.2.0")
  ],
  targets: [
    .binaryTarget(
      name: "medialayerFFI",
      url: "https://github.com/noma4i/medialayer/releases/download/3.54.0/MediaLayerFFI.xcframework.zip",
      checksum: "ffc5538a9154ce7f8010be4d9e15dde9fccff2f8a32d6874d3ab5f63820a8360"
    ),
    .target(
      name: "MediaLayer",
      dependencies: ["medialayerFFI"],
      path: "MediaLayer/Sources/MediaLayer",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .target(
      name: "MediaLayerUI",
      dependencies: ["MediaLayer", .product(name: "Nuke", package: "Nuke")],
      path: "MediaLayer/Sources/MediaLayerUI",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .target(
      name: "MediaLayerTesting",
      dependencies: ["MediaLayer", "MediaLayerUI"],
      path: "MediaLayer/Sources/MediaLayerTesting",
      swiftSettings: [.swiftLanguageMode(.v5)]
    )
  ]
)
