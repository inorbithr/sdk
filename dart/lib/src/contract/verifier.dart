import 'dart:convert';

import '../generated/contract.g.dart';
import '../version.dart';
import 'mismatch.dart';

/// One operation of the contract.
final class ContractOperation {
  const ContractOperation._(
    this.id,
    this.method,
    this.path,
    this._answers,
    this._pattern,
  );

  /// The operation id, such as `ListMonitors`.
  final String id;

  /// The HTTP method, upper case.
  final String method;

  /// The path template, such as `/v1/accounts/orgs/{org_id}/monitors`.
  final String path;

  final Map<String, Object?> _answers;
  final RegExp _pattern;

  /// Literal characters in the template: a more literal template wins a tie
  /// (`/v1/labs/assignments` over `/v1/labs/{lab_id}`).
  int get _literal => path.replaceAll(RegExp(r'\{[^}]+\}'), '').length;
}

/// The result of checking one answer.
final class ContractCheck {
  const ContractCheck._(this.operation, this.mismatches);

  /// The operation the call matched, or null when the contract has no such
  /// operation (the answer was not checked).
  final ContractOperation? operation;

  /// What differed; empty when the answer keeps the contract or was not checked.
  final List<ContractMismatch> mismatches;

  /// Whether the contract covers this call.
  bool get covered => operation != null;
}

/// Checks API answers against the published contract (platform RFC 0074.8).
///
/// The check is shallow on purpose and cheap: required fields are present, each value
/// has the kind its schema says (64-bit integers may come as strings, unset timestamps
/// as `""`), listed values are among the listed ones, and the first [maxItems] items of
/// each list are checked. A field the contract does not know is allowed: the API may
/// add fields without a new version.
final class ContractVerifier {
  /// A verifier over a contract map (the shape `tool/gen_contract.dart` writes).
  ContractVerifier.fromContract(
    Map<String, Object?> contract, {
    this.mode = ContractMode.record,
    this.maxItems = 50,
  }) : apiVersion = '${contract['api_version'] ?? ''}',
       _schemas = (contract['schemas'] as Map<String, Object?>?) ?? const {},
       _operations = [
         for (final o in (contract['operations'] as List<Object?>?) ?? const [])
           _operation(o! as Map<String, Object?>),
       ];

  /// The contract this package was built with.
  factory ContractVerifier.bundled({ContractMode mode = ContractMode.record}) =>
      ContractVerifier.fromContract(
        _bundled ??= jsonDecode(contractJson) as Map<String, Object?>,
        mode: mode,
      );

  static Map<String, Object?>? _bundled;

  /// What happens on a mismatch.
  final ContractMode mode;

  /// How many items of each list are checked.
  final int maxItems;

  /// The contract's API version.
  final String apiVersion;

  final Map<String, Object?> _schemas;
  final List<ContractOperation> _operations;

  /// Every operation the contract knows.
  List<ContractOperation> get operations => List.unmodifiable(_operations);

  static ContractOperation _operation(Map<String, Object?> o) {
    final path = o['path']! as String;
    final pattern = RegExp(
      '^${path.split('/').map((s) => s.startsWith('{') ? '[^/]+' : RegExp.escape(s)).join('/')}\$',
    );
    return ContractOperation._(
      o['id']! as String,
      o['method']! as String,
      path,
      (o['answers'] as Map<String, Object?>?) ?? const {},
      pattern,
    );
  }

  /// The operation a call to [method] [path] is, or null. [path] is the request
  /// path without the query and without any base path in front of `/v1`.
  ContractOperation? operationFor(String method, String path) {
    final m = method.toUpperCase();
    ContractOperation? best;
    for (final op in _operations) {
      if (op.method != m || !op._pattern.hasMatch(path)) continue;
      if (best == null || op._literal > best._literal) best = op;
    }
    return best;
  }

  /// Check one answer. In [ContractMode.strict] a mismatch throws
  /// [ContractViolation]; otherwise the mismatches are returned.
  ContractCheck verify({
    required String method,
    required String path,
    required int status,
    required Object? body,
  }) {
    final op = operationFor(method, path);
    if (op == null) return const ContractCheck._(null, []);
    final schema = op._answers['$status'];
    if (schema == null) return ContractCheck._(op, const []);
    final found = <String, ContractMismatch>{};
    void add(String field, String expected, String got) {
      final m = ContractMismatch(
        operation: op.id,
        method: op.method,
        path: op.path,
        field: field,
        expected: expected,
        got: got,
        apiVersion: apiVersion,
        sdkVersion: sdkVersion,
      );
      found.putIfAbsent(m.key, () => m);
    }

    _check(schema, body, '', add, 0);
    final list = found.values.toList();
    if (list.isNotEmpty && mode == ContractMode.strict) {
      throw ContractViolation(list);
    }
    return ContractCheck._(op, list);
  }

  void _check(
    Object? schema,
    Object? value,
    String at,
    void Function(String field, String expected, String got) add,
    int depth,
  ) {
    if (depth > 32 || schema is! Map<String, Object?>) return;
    final ref = schema[r'$ref'];
    if (ref is String) {
      _check(_schemas[ref.split('/').last], value, at, add, depth + 1);
      return;
    }
    final all = schema['allOf'];
    if (all is List) {
      for (final s in all) {
        _check(s, value, at, add, depth + 1);
      }
    }
    final any = schema['oneOf'] ?? schema['anyOf'];
    if (any is List && any.isNotEmpty) {
      var fits = false;
      for (final s in any) {
        var clean = true;
        _check(s, value, at, (_, _, _) => clean = false, depth + 1);
        if (clean) {
          fits = true;
          break;
        }
      }
      if (!fits) add(at, 'one of ${any.length} shapes', _kind(value));
      return;
    }

    final types = switch (schema['type']) {
      final String t => [t],
      final List<Object?> l => [for (final t in l) '$t'],
      _ => const <String>[],
    };
    if (types.isNotEmpty &&
        !types.any((t) => _fits(t, schema['format'], value))) {
      add(at, types.join(' or '), _kind(value));
      return;
    }

    final listed = schema['enum'];
    if (listed is List && value != null && !listed.contains(value)) {
      add(at, 'one of the listed values', 'unlisted value');
    }

    if (value is Map) {
      final required = schema['required'];
      if (required is List) {
        for (final name in required) {
          if (!value.containsKey(name)) add('$at/$name', 'present', 'missing');
        }
      }
      final props = schema['properties'];
      if (props is Map<String, Object?>) {
        for (final e in props.entries) {
          if (value.containsKey(e.key)) {
            _check(e.value, value[e.key], '$at/${e.key}', add, depth + 1);
          }
        }
      }
      final extra = schema['additionalProperties'];
      if (extra is Map<String, Object?> && props is! Map) {
        var n = 0;
        for (final v in value.values) {
          if (n++ >= maxItems) break;
          _check(extra, v, '$at/*', add, depth + 1);
        }
      }
    } else if (value is List) {
      final items = schema['items'];
      var n = 0;
      for (final v in value) {
        if (n++ >= maxItems) break;
        _check(items, v, '$at/*', add, depth + 1);
      }
    }
  }

  static final _digits = RegExp(r'^-?\d+$');

  static bool _fits(String type, Object? format, Object? value) =>
      switch (type) {
        'null' => value == null,
        'boolean' => value is bool,
        // 64-bit integers travel as strings on the wire (docs/design.md).
        'integer' =>
          (value is num && value == value.truncateToDouble()) ||
              (format == 'int64' && value is String && _digits.hasMatch(value)),
        // Proto3 JSON writes non-finite doubles as strings.
        'number' =>
          value is num ||
              (value is String &&
                  const {'NaN', 'Infinity', '-Infinity'}.contains(value)),
        'string' => value is String,
        'array' => value is List,
        'object' => value is Map,
        _ => true,
      };

  static String _kind(Object? v) => switch (v) {
    null => 'null',
    bool() => 'boolean',
    int() => 'integer',
    double() => 'number',
    String() => 'string',
    List() => 'array',
    Map() => 'object',
    _ => 'unknown',
  };
}
