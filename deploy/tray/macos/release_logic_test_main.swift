import Foundation

func expect(_ condition: @autoclosure () -> Bool, _ message: String) {
    if !condition() {
        fputs("FAIL: \(message)\n", stderr)
        exit(1)
    }
}

let fixture = """
{
  "tag_name": "v0.5.0",
  "assets": [
    {"name":"walgit-0.5.0-x86_64.dmg","browser_download_url":"https://example.invalid/x86.dmg","digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
    {"name":"walgit-0.5.0-arm64.dmg","browser_download_url":"https://example.invalid/arm.dmg","digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
  ]
}
""".data(using: .utf8)!

do {
    let info = try parseLatestRelease(fixture, arch: "arm64")
    expect(info.tag == "v0.5.0", "tag")
    expect(info.version == "0.5.0", "version")
    expect(info.asset.name == "walgit-0.5.0-arm64.dmg", "arch-specific asset")
    expect(info.asset.sha256 == String(repeating: "b", count: 64), "digest")
    expect(isVersionNewer("0.5.0", than: "0.4.0"), "newer patch line")
    expect(!isVersionNewer("0.4.0", than: "0.4.0"), "same version")
    expect(!isVersionNewer("0.4.9", than: "0.5.0"), "older version")
    expect(isVersionNewer("0.5.0", than: "0.5.0-beta.1"), "release beats prerelease")
    expect(isVersionNewer("0.5.0-beta.2", than: "0.5.0-beta.1"), "newer prerelease")
    expect(isVersionNewer("1.0.0", than: "0.99.99"), "major")
    expect(isVersionNewer("0.5.0-beta.10", than: "0.5.0-beta.2"), "numeric prerelease ordering")
    expect(!isVersionNewer("0.5.0-rc.1", than: "0.5.0"), "release beats rc")
    expect(isVersionNewer("0.5.0", than: "0.5.0-rc.1"), "rc is older than release")
    expect(!isVersionNewer("0.5", than: "0.5.0"), "trailing zero equal")
    expect(!isVersionNewer("0.5.0", than: "0.5"), "trailing zero equal reversed")
    expect(isVersionNewer("V0.6.0", than: "v0.5.0"), "uppercase v prefix")
    expect(!isVersionNewer("", than: "0.5.0"), "empty version is not newer")
} catch {
    fputs("FAIL: \(error)\n", stderr)
    exit(1)
}

func expectReleaseError(_ json: String, arch: String, _ message: String) {
    do {
        _ = try parseLatestRelease(Data(json.utf8), arch: arch)
        fputs("FAIL: expected error but parsed: \(message)\n", stderr)
        exit(1)
    } catch {
    }
}

expectReleaseError("{}", arch: "arm64", "empty object")
expectReleaseError(#"{"tag_name":""}"#, arch: "arm64", "empty tag")

let missingDigest = #"""
{"tag_name":"v0.5.0","assets":[{"name":"walgit-0.5.0-arm64.dmg","browser_download_url":"https://example.invalid/a.dmg"}]}
"""#
expectReleaseError(missingDigest, arch: "arm64", "asset without digest")

let shortDigest = #"""
{"tag_name":"v0.5.0","assets":[{"name":"walgit-0.5.0-arm64.dmg","browser_download_url":"https://example.invalid/a.dmg","digest":"sha256:deadbeef"}]}
"""#
expectReleaseError(shortDigest, arch: "arm64", "truncated digest")

let noArchAsset = #"""
{"tag_name":"v0.5.0","assets":[{"name":"walgit-0.5.0-x86_64.dmg","browser_download_url":"https://example.invalid/x.dmg","digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}]}
"""#
expectReleaseError(noArchAsset, arch: "arm64", "missing arm64 asset")

print("release logic tests: ok")
