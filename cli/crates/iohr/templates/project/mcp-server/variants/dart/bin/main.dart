// Starts the server over stdio, the way a host starts it. No port, no credential.
import 'dart:io' as io;

import 'package:dart_mcp/stdio.dart';
import 'package:@@MODULE@@/server.dart';

void main() {
  WordServer(stdioChannel(input: io.stdin, output: io.stdout));
}
