// One test, no test package to fetch: exits 1 when it fails.
import 'dart:io';

import '../bin/main.dart';

void main() {
  if (hello() != 'hello from @@NAME@@') {
    stderr.writeln('FAIL: hello() returned ${hello()}');
    exit(1);
  }
  stdout.writeln('ok: hello');
}
