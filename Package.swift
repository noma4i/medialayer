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
      url: "https://github.com/noma4i/medialayer/releases/download/3.41.1/MediaLayerFFI.xcframework.zip",
      checksum: "63a6a2f2ec7f177b30e2ec824e3238bca8ad7e3c824a834c08ddd88d9194bef7"
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
