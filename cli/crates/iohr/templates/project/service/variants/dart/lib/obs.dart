// Observability on the Dart SDK alone: JSON logs, spans that follow the W3C traceparent
// header, and request counters in the Prometheus text format.
//
// Nothing here records a header, a query string or an environment value: a log line and a span
// name the route that matched, the status, the duration and the trace id.
import 'dart:convert';
import 'dart:math';

final _traceparent = RegExp(r'^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$');
final _random = Random.secure();

String _hex(int bytes) =>
    List.generate(bytes, (_) => _random.nextInt(256).toRadixString(16).padLeft(2, '0')).join();

/// One JSON object per line to a sink.
class Logger {
  Logger(this._sink);
  final void Function(String line) _sink;

  void log(String level, String msg, [Map<String, Object> fields = const {}]) {
    _sink(jsonEncode({'ts': DateTime.now().toUtc().toIso8601String(), 'level': level, 'msg': msg, ...fields}));
  }

  void info(String msg, [Map<String, Object> fields = const {}]) => log('INFO', msg, fields);
  void error(String msg, [Map<String, Object> fields = const {}]) => log('ERROR', msg, fields);
}

/// One request's trace: the id it joined (or started) and its own id.
class Span {
  Span._(this.traceId, this.parentId);

  /// Joins the trace of a valid traceparent header, or starts a new one.
  factory Span.start(String? header) {
    final m = _traceparent.firstMatch(header ?? '');
    if (m != null && !RegExp(r'^0+$').hasMatch(m[1]!) && !RegExp(r'^0+$').hasMatch(m[2]!)) {
      return Span._(m[1]!, m[2]);
    }
    return Span._(_hex(16), null);
  }

  final String traceId;
  final String? parentId;
  final String spanId = _hex(8);
  final Stopwatch _clock = Stopwatch()..start();

  /// The traceparent to send back (and on to callers): this span is the parent.
  String get header => '00-$traceId-$spanId-01';

  double get millis => (_clock.elapsedMicroseconds / 1000 * 1000).round() / 1000;
}

/// Request counts by route and status. Routes are matched patterns, never raw paths, so the
/// label set is bounded by the code and holds no user input.
class Metrics {
  final Map<String, int> _counts = {};

  void inc(String route, int status) {
    final key = 'route="$route",status="$status"';
    _counts[key] = (_counts[key] ?? 0) + 1;
  }

  String render() {
    final b = StringBuffer()
      ..writeln('# HELP http_requests_total Requests served, by route and status.')
      ..writeln('# TYPE http_requests_total counter');
    for (final key in _counts.keys.toList()..sort()) {
      b.writeln('http_requests_total{$key} ${_counts[key]}');
    }
    return b.toString();
  }
}
