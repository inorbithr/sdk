import 'dart:io';

import 'package:@@MODULE@@/cli.dart';

void main(List<String> args) {
  // Passes the real streams; the exit code is the result of run().
  exitCode = run(args, stdout.write, stderr.write);
}
