// iohr's Secure Enclave helper (RFC 0113). The iohr binary is Rust with `unsafe` forbidden, and
// a plain command line tool cannot keep a Secure Enclave key in the keychain (no entitlement),
// so this small program does the two things only CryptoKit can: make a P-256 key that lives in
// the enclave, and sign with it after Touch ID. The key never leaves the enclave; what is
// written to disk is CryptoKit's opaque blob, which only this Mac's enclave can use.
//
//   helper create BLOB          make a key behind Touch ID; BLOB gets the blob (0600), stdout
//                               the public key (base64url of the 65-byte point)
//   helper public BLOB          the public key of the key in BLOB
//   helper sign BLOB REASON     read the message on stdin, ask for Touch ID with REASON,
//                               print the signature (base64url, DER, over SHA-256)
//
// The key is bound to the fingerprints enrolled now (`.biometryCurrentSet`): add or remove a
// finger and it stops working, and the owner enrols a new one.
import CryptoKit
import Foundation
import LocalAuthentication

func fail(_ text: String) -> Never {
    FileHandle.standardError.write(Data((text + "\n").utf8))
    exit(1)
}

func b64url(_ data: Data) -> String {
    data.base64EncodedString()
        .replacingOccurrences(of: "+", with: "-")
        .replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
}

let args = CommandLine.arguments
guard args.count >= 3 else { fail("usage: helper create|public|sign BLOB [REASON]") }
let mode = args[1]
let blob = URL(fileURLWithPath: args[2])

guard SecureEnclave.isAvailable else { fail("this Mac has no Secure Enclave") }

func load(_ reason: String?) -> SecureEnclave.P256.Signing.PrivateKey {
    guard let data = try? Data(contentsOf: blob) else { fail("cannot read the key file") }
    let context = LAContext()
    if let reason = reason { context.localizedReason = reason }
    do {
        return try SecureEnclave.P256.Signing.PrivateKey(
            dataRepresentation: data, authenticationContext: context)
    } catch {
        fail("the key cannot be used (fingerprints changed? enrol a new key): \(error)")
    }
}

switch mode {
case "create":
    if FileManager.default.fileExists(atPath: blob.path) { fail("a key file is already there") }
    var error: Unmanaged<CFError>?
    guard
        let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            [.privateKeyUsage, .biometryCurrentSet], &error)
    else { fail("cannot make the access rule: \(String(describing: error))") }
    do {
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
        guard
            FileManager.default.createFile(
                atPath: blob.path, contents: key.dataRepresentation,
                attributes: [.posixPermissions: 0o600])
        else { fail("cannot write the key file") }
        print(b64url(key.publicKey.x963Representation))
    } catch {
        fail("cannot make the key: \(error)")
    }
case "public":
    print(b64url(load(nil).publicKey.x963Representation))
case "sign":
    guard args.count >= 4 else { fail("sign needs a reason") }
    let message = FileHandle.standardInput.readDataToEndOfFile()
    let key = load(args[3])
    do {
        print(b64url(try key.signature(for: message).derRepresentation))
    } catch {
        fail("signing was refused: \(error)")
    }
default:
    fail("unknown mode")
}
