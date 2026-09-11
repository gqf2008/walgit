import Foundation

struct ReleaseAsset: Equatable {
    let name: String
    let url: URL
    let sha256: String
}

struct ReleaseInfo: Equatable {
    let tag: String
    let version: String
    let asset: ReleaseAsset
}

enum ReleaseLogicError: Error, CustomStringConvertible {
    case malformed(String)
    case missingAsset(String)
    case invalidDigest(String)

    var description: String {
        switch self {
        case let .malformed(message): "malformed release JSON: \(message)"
        case let .missingAsset(name): "release has no macOS asset \(name)"
        case let .invalidDigest(name): "release asset \(name) has no sha256 digest"
        }
    }
}

func stripVersionPrefix(_ value: String) -> String {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    return trimmed.hasPrefix("v") || trimmed.hasPrefix("V")
        ? String(trimmed.dropFirst())
        : trimmed
}

func releaseArch() -> String {
    #if arch(arm64)
    return "arm64"
    #elseif arch(x86_64)
    return "x86_64"
    #else
    return "unknown"
    #endif
}

func parseLatestRelease(_ data: Data, arch: String = releaseArch()) throws -> ReleaseInfo {
    let raw = try JSONSerialization.jsonObject(with: data)
    guard let object = raw as? [String: Any] else {
        throw ReleaseLogicError.malformed("root is not an object")
    }
    guard let tag = object["tag_name"] as? String, !tag.isEmpty else {
        throw ReleaseLogicError.malformed("missing tag_name")
    }
    let version = stripVersionPrefix(tag)
    guard !version.isEmpty else {
        throw ReleaseLogicError.malformed("empty release version")
    }
    guard let assets = object["assets"] as? [[String: Any]] else {
        throw ReleaseLogicError.malformed("missing assets array")
    }
    let expected = "walgit-\(version)-\(arch).dmg"
    var fallback: [String: Any]?
    for asset in assets {
        guard let name = asset["name"] as? String, name.hasSuffix(".dmg") else { continue }
        if name == expected {
            fallback = asset
            break
        }
        if name.hasSuffix("-\(arch).dmg") {
            fallback = asset
        }
    }
    guard let asset = fallback,
          let name = asset["name"] as? String,
          let urlString = asset["browser_download_url"] as? String,
          let url = URL(string: urlString)
    else {
        throw ReleaseLogicError.missingAsset(expected)
    }
    guard let digest = asset["digest"] as? String, digest.hasPrefix("sha256:") else {
        throw ReleaseLogicError.invalidDigest(name)
    }
    let sha = String(digest.dropFirst("sha256:".count)).lowercased()
    guard sha.count == 64, sha.allSatisfy({ $0.isHexDigit }) else {
        throw ReleaseLogicError.invalidDigest(name)
    }
    return ReleaseInfo(tag: tag, version: version, asset: ReleaseAsset(name: name, url: url, sha256: sha))
}

private func versionParts(_ value: String) -> ([Int], [String]) {
    let normalized = stripVersionPrefix(value)
    let pieces = normalized.split(separator: "-", maxSplits: 1, omittingEmptySubsequences: false)
    let numeric = pieces.first.map(String.init) ?? normalized
    let prerelease = pieces.count > 1 ? String(pieces[1]) : ""
    let numbers = numeric.split(separator: ".", omittingEmptySubsequences: false).map {
        Int($0) ?? 0
    }
    let pre = prerelease.isEmpty
        ? []
        : prerelease.split(separator: ".", omittingEmptySubsequences: false).map(String.init)
    return (numbers, pre)
}

private func comparePrerelease(_ lhs: [String], _ rhs: [String]) -> ComparisonResult {
    for index in 0..<max(lhs.count, rhs.count) {
        if index >= lhs.count { return .orderedAscending }
        if index >= rhs.count { return .orderedDescending }
        let left = lhs[index]
        let right = rhs[index]
        if let li = Int(left), let ri = Int(right), li != ri {
            return li < ri ? .orderedAscending : .orderedDescending
        }
        let result = left.compare(right, options: [.numeric, .caseInsensitive])
        if result != .orderedSame { return result }
    }
    return .orderedSame
}

func isVersionNewer(_ candidate: String, than current: String) -> Bool {
    let (candidateNumbers, candidatePre) = versionParts(candidate)
    let (currentNumbers, currentPre) = versionParts(current)
    let width = max(candidateNumbers.count, currentNumbers.count)
    for index in 0..<width {
        let left = index < candidateNumbers.count ? candidateNumbers[index] : 0
        let right = index < currentNumbers.count ? currentNumbers[index] : 0
        if left != right { return left > right }
    }
    if candidatePre.isEmpty != currentPre.isEmpty {
        return currentPre.isEmpty == false
    }
    if candidatePre.isEmpty && currentPre.isEmpty { return false }
    return comparePrerelease(candidatePre, currentPre) == .orderedDescending
}

func fixtureReleaseCheck(path: String, currentVersion: String, arch: String = releaseArch()) throws -> ReleaseInfo {
    let data = try Data(contentsOf: URL(fileURLWithPath: path))
    return try parseLatestRelease(data, arch: arch)
}
