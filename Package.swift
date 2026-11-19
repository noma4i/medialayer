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
      url: "https://github.com/noma4i/medialayer/releases/download/3.48.0/MediaLayerFFI.xcframework.zip",
      checksum: "08b18533dbded063f5f6c6d2bb30986ad1198c11104004472bcb0a8aeb0dc8b0"
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
