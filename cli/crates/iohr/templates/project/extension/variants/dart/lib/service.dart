/// The service and its token channel, for the program and for the self-test.
library;

export 'src/channel.dart' show Refused, token;
export 'src/service.dart' show heartbeat, id, run, scopes, serve, version;
