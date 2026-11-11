// Reads HOST and PORT from the environment, serves, and stops cleanly on SIGINT or SIGTERM.
import 'dart:async';
import 'dart:io';

import 'package:@@MODULE@@/obs.dart';
import 'package:@@MODULE@@/service.dart';

Future<void> main() async {
  final log = Logger(stdout.writeln);
  final ({String host, int port}) cfg;
  try {
    cfg = loadConfig(Platform.environment);
  } on FormatException catch (e) {
    log.error('bad configuration', {'error': e.message});
    exit(2);
  }
  final svc = Service(log);
  try {
    await svc.start(cfg.host, cfg.port);
  } on SocketException {
    log.error('cannot listen');
    exit(1);
  }
  log.info('listening', {'addr': '${cfg.host}:${svc.port}'});

  var stopping = false;
  Future<void> stop(ProcessSignal _) async {
    if (stopping) return;
    stopping = true;
    log.info('shutting down');
    // Ready goes false first, then the server closes; a hard stop follows after 10 seconds.
    final guard = Timer(const Duration(seconds: 10), () => exit(1));
    await svc.stop();
    guard.cancel();
    exit(0);
  }

  ProcessSignal.sigint.watch().listen(stop);
  ProcessSignal.sigterm.watch().listen(stop);
}
