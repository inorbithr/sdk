/// One way an answer differed from the published contract.
///
/// It names where and what kind of difference, never a value: no field content, no
/// identifier, no token can reach a log or a report through it.
final class ContractMismatch {
  /// Built by the verifier.
  const ContractMismatch({
    required this.operation,
    required this.method,
    required this.path,
    required this.field,
    required this.expected,
    required this.got,
    required this.apiVersion,
    required this.sdkVersion,
  });

  /// The contract's operation id, such as `ListMonitors`.
  final String operation;

  /// The HTTP method.
  final String method;

  /// The contract's path template, such as `/v1/accounts/orgs/{org_id}/monitors`;
  /// never the concrete path, which carries identifiers.
  final String path;

  /// A JSON pointer to the field, with every array index written `*`
  /// (`/monitors/*/interval_secs`); `` (empty) is the whole answer.
  final String field;

  /// What the contract says: a kind (`string`, `integer`, `object`), `present`, or
  /// `one of the listed values`.
  final String expected;

  /// What came instead, as a kind: `missing`, `null`, `number`, `unlisted value`.
  final String got;

  /// The API version of the contract the answer was checked against.
  final String apiVersion;

  /// This package's version.
  final String sdkVersion;

  /// The same mismatch seen again on another call compares equal.
  String get key => '$method $path $field $expected $got';

  /// A map safe to show, export or send: only the fields above.
  Map<String, String> toJson() => {
    'operation': operation,
    'method': method,
    'path': path,
    'field': field,
    'expected': expected,
    'got': got,
    'api_version': apiVersion,
    'sdk_version': sdkVersion,
  };

  @override
  bool operator ==(Object other) =>
      other is ContractMismatch && other.key == key;

  @override
  int get hashCode => key.hashCode;

  @override
  String toString() =>
      '$operation ($method $path): ${field.isEmpty ? 'the answer' : field} '
      'should be $expected, got $got';
}

/// Thrown in [ContractMode.strict] when an answer breaks the contract.
final class ContractViolation implements Exception {
  /// The mismatches of one answer.
  const ContractViolation(this.mismatches);

  /// Every mismatch found in the answer, at least one.
  final List<ContractMismatch> mismatches;

  @override
  String toString() =>
      'the answer breaks the published contract:\n${mismatches.join('\n')}';
}

/// What the verifier does when an answer breaks the contract.
enum ContractMode {
  /// Return the mismatches and let the caller decode what is valid. The default:
  /// an app keeps working when the API drifts.
  record,

  /// Throw [ContractViolation]. For tests and the conformance suite.
  strict,
}
