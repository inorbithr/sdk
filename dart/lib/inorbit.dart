/// Dart runtime for the InOrbit public API.
///
/// Today it holds the published contract and checks answers against it
/// ([ContractVerifier]); the generated client follows (platform RFC 0074.8).
library;

export 'src/contract/mismatch.dart';
export 'src/contract/verifier.dart';
export 'src/version.dart';
