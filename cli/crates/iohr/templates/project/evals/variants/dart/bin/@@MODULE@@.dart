import 'dart:io';

import 'package:@@MODULE@@/@@MODULE@@.dart';

/// @@NAME@@: an offline eval suite with a pass bar. Dart core libraries only, no network.
/// Exit 0 when the bar is met, 1 when missed, 2 when the dataset or the bar is wrong.
void main(List<String> args) {
  exitCode = runSuite(args, stdout, stderr);
}
