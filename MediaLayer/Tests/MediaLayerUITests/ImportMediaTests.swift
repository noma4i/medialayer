import CryptoKit
import Foundation
@testable import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

private func open() async throws -> MediaLayer {
  try await MediaLayer.open(
    fetcher: TestFetcher(),
    settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
    directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  )
}

extension GlobalMedia {
  @Suite(.serialized)
  struct ImportMediaTests {
    @Test
    func inv_import_01_imported_image_and_video_are_accepted_by_preparation() async throws {
      let media = try await open()
      let preparer = MediaPreparer()
      let image = try mediaPrepareDescriptor(intent: .image(edit: PhotoEdit(maxSize: nil, enlarge: false, crop: nil, outputSize: nil, mirror: false, rotationDegrees: 0), quality: nil, format: .jpeg))
      let video = try mediaPrepareDescriptor(intent: .video(limits: nil, mirror: false, onCannotDecode: .fail))
      for (name, ext, descriptor) in [("exif6_editor", "jpg", image), ("silent", "mp4", video)] {
        let source = Bundle.module.url(forResource: name, withExtension: ext, subdirectory: "Fixtures")!
        let imported = try await media.importMedia(source)
        defer { try? FileManager.default.removeItem(at: imported.url) }
        #expect(imported.isNew)
        #expect(imported.url != source)
        let destination = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).\(ext)")
        defer { try? FileManager.default.removeItem(at: destination) }
        let outcome = await preparer.run(job: MediaPrepareJob(attemptId: 1, source: imported.url.path, destination: destination.path, descriptor: descriptor)) { _ in }
        switch outcome {
        case .done, .unchanged: break
        default: Issue.record("the imported \(name) was not accepted: \(outcome)")
        }
        #expect(FileManager.default.fileExists(atPath: imported.url.path))
      }
    }

    @Test
    func inv_import_02_large_file_preserves_the_sha256() async throws {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      let source = directory.appendingPathComponent("large.mov")
      FileManager.default.createFile(atPath: source.path, contents: nil)
      let writer = try FileHandle(forWritingTo: source)
      let chunk = Data(repeating: 0x97, count: 64 * 1024)
      var hash = SHA256()
      for _ in 0..<256 { try writer.write(contentsOf: chunk); hash.update(data: chunk) }
      try writer.close()
      var requests: [Int] = []
      var progress: [ImportProgress] = []
      let imported = try importMediaFile(source, directory: directory, progress: { progress.append($0) }, read: { handle, count in
        requests.append(count)
        return try handle.read(upToCount: count)
      })
      let reader = try FileHandle(forReadingFrom: imported.url)
      defer { try? reader.close() }
      var copied = SHA256()
      while let bytes = try reader.read(upToCount: 64 * 1024), !bytes.isEmpty { copied.update(data: bytes) }
      #expect(hash.finalize() == copied.finalize())
      #expect(imported.url.pathExtension == "mov")
      #expect(requests.count > 256)
      #expect(requests.allSatisfy { $0 <= 64 * 1024 })
      let size: Int64 = 16 * 1024 * 1024
      let first = try #require(progress.first)
      let last = try #require(progress.last)
      #expect(first.bytesCopied == 0)
      #expect(last.bytesCopied == size)
      #expect(progress.allSatisfy { $0.totalBytes == size })
      #expect(zip(progress, progress.dropFirst()).allSatisfy { $0.0.bytesCopied <= $0.1.bytesCopied })
      #expect(try FileManager.default.attributesOfItem(atPath: imported.url.path)[.size] as? Int == 16 * 1024 * 1024)
    }

    @Test
    func inv_import_03_open_read_and_write_errors_are_explicit() throws {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      #expect(throws: MediaError.self) { try importMediaFile(directory) }
      let source = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!
      try Data().write(to: directory)
      #expect(throws: MediaError.self) { try importMediaFile(source, directory: directory) }
      try FileManager.default.removeItem(at: directory)
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      var input: FileHandle?
      var output: FileHandle?
      #expect(throws: MediaError.self) {
        try importMediaFile(source, directory: directory, read: { handle, _ in
          input = handle
          throw CocoaError(.fileReadUnknown)
        })
      }
      #expect(throws: (any Error).self) { try input!.offset() }
      #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
      #expect(throws: MediaError.self) {
        try importMediaFile(source, directory: directory, write: { handle, bytes in
          output = handle
          try handle.write(contentsOf: bytes.prefix(1))
          throw CocoaError(.fileWriteOutOfSpace)
        })
      }
      #expect(throws: (any Error).self) { try output!.offset() }
      #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
    }

    @Test
    func inv_import_04_cancellation_closes_streams_and_removes_only_the_partial_file() async throws {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      let source = directory.appendingPathComponent("source.mp4")
      try Data(repeating: 0x97, count: 128 * 1024).write(to: source)
      let finished = try importMediaFile(source, directory: directory)
      let attempt = Task.detached { () throws -> Bool in
        var input: FileHandle?
        var output: FileHandle?
        var reads = 0
        do {
          _ = try importMediaFile(source, directory: directory, read: { handle, count in
            input = handle
            reads += 1
            return try handle.read(upToCount: count)
          }, write: { handle, bytes in
            output = handle
            try handle.write(contentsOf: bytes)
            withUnsafeCurrentTask { $0?.cancel() }
          })
          Issue.record("a cancelled copy returned a file")
        } catch is CancellationError {
          #expect(reads == 1)
          #expect(throws: (any Error).self) { try input!.offset() }
          #expect(throws: (any Error).self) { try output!.offset() }
          return true
        }
        return false
      }
      #expect(try await attempt.value)
      #expect(Set(try FileManager.default.contentsOfDirectory(atPath: directory.path)) == Set([source.lastPathComponent, finished.url.lastPathComponent]))
      #expect(try Data(contentsOf: finished.url) == Data(contentsOf: source))
    }

    @Test
    func custom_directory_is_created_and_receives_the_copy() async throws {
      let media = try await open()
      let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      defer { try? FileManager.default.removeItem(at: root) }
      let source = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!
      let completedDirectory = root.appendingPathComponent("completed/nested")
      let imported = try await media.importMedia(source, directory: completedDirectory)
      #expect(imported.url.deletingLastPathComponent() == completedDirectory)
      #expect(try Data(contentsOf: imported.url) == Data(contentsOf: source))
    }
  }
}
