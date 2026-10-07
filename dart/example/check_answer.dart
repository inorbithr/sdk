// Check one API answer against the published contract. The README quotes this file;
// `mise run dart:check` analyses it.
import 'package:inorbit/inorbit.dart';

void main() {
  final verifier = ContractVerifier.bundled();

  // An answer as an app decoded it; this one forgot a required field.
  final decodedJson = <String, Object?>{'monitors': <Object?>[]};

  final check = verifier.verify(
    method: 'GET',
    // The request path, without the query.
    path: '/v1/accounts/orgs/o-1/monitors',
    status: 200,
    body: decodedJson,
  );
  for (final m in check.mismatches) {
    // ConnectionsService.ListMonitors (GET /v1/accounts/orgs/{org_id}/monitors):
    // /next_page_token should be present, got missing
    print(m);
  }
}
