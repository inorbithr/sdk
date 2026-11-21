import 'dart:io';

import 'package:@@MODULE@@/service.dart';

/// @@ID@@: the demo project's private macOS service. See lib/src/service.dart.
Future<void> main() async {
  // exit() on purpose: the heartbeat timer must not keep the process alive after a stop.
  exit(await run(Platform.environment));
}
