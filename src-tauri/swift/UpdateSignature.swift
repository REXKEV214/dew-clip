import Foundation
import Security

private func staticCode(_ path: UnsafePointer<CChar>) -> SecStaticCode? {
    let url = URL(fileURLWithPath: String(cString: path))
    var code: SecStaticCode?
    guard SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess else { return nil }
    return code
}

@_cdecl("dewclip_check_update_bundle")
public func checkUpdateBundle(_ path: UnsafePointer<CChar>, _ version: UnsafePointer<CChar>) -> Bool {
    let url = URL(fileURLWithPath: String(cString: path)).appendingPathComponent("Contents/Info.plist")
    guard let data = try? Data(contentsOf: url),
          let info = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
    else { return false }
    return info["CFBundleIdentifier"] as? String == "io.github.rexkev214.dewclip"
        && info["CFBundleShortVersionString"] as? String == String(cString: version)
}

@_cdecl("dewclip_update_requirement")
public func updateRequirement(_ candidatePath: UnsafePointer<CChar>, _ referencePath: UnsafePointer<CChar>?) -> Int32 {
    guard let candidate = staticCode(candidatePath) else { return -1 }
    let reference: SecStaticCode
    var requirement: SecRequirement?
    if let referencePath {
        guard let code = staticCode(referencePath),
              SecCodeCopyDesignatedRequirement(code, [], &requirement) == errSecSuccess
        else { return -1 }
        reference = code
    } else {
        var running: SecCode?
        var runningStatic: SecStaticCode?
        guard SecCodeCopySelf([], &running) == errSecSuccess, let running,
              SecCodeCopyStaticCode(running, [], &runningStatic) == errSecSuccess, let runningStatic,
              SecCodeCopyDesignatedRequirement(runningStatic, [], &requirement) == errSecSuccess
        else { return -1 }
        reference = runningStatic
    }
    guard let requirement else { return -1 }
    if referencePath == nil {
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(reference, [], &info) == errSecSuccess,
              let signing = info as? [String: Any],
              let flags = signing[kSecCodeInfoFlags as String] as? NSNumber
        else { return -1 }
        if SecCodeSignatureFlags(rawValue: flags.uint32Value).contains(.adhoc) { return 0 }
    }
    let result = SecStaticCodeCheckValidity(candidate, SecCSFlags(rawValue: kSecCSUseAllArchitectures), requirement)
    if result == errSecSuccess { return 1 }
    return result == errSecCSReqFailed ? 0 : -1
}
