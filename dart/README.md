# inorbit (Dart)

The Dart runtime for the [InOrbit API](https://docs.inorbit.hr). It is started, not
finished: today it holds the published contract and checks the API's answers against it,
so an app finds out when an answer drifts from what the contract says. The client
follows (platform RFC 0074.8).

Not on pub.dev yet. Depend on it from git:

```yaml
dependencies:
  inorbit:
    git:
      url: https://github.com/inorbithr/sdk.git
      path: dart
      ref: main
```

## Check an answer

From [`example/check_answer.dart`](example/check_answer.dart):

```dart
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
```

- `check.covered` is false when the contract has no such operation; nothing was checked.
- A mismatch names the operation, the path template, the field and two kinds. It never
  holds a value, an identifier or the concrete path, so it is safe to log or report.
- `ContractVerifier.bundled(mode: ContractMode.strict)` throws `ContractViolation`
  instead, for tests.

What is checked: required fields are present; every value has the kind its schema says
(64-bit integers may arrive as strings, unset timestamps as `""`); a listed value is one
of the listed ones; the first 50 items of each list. A field the contract does not know
is allowed, since the API may add fields.

## Check the live API

```sh
mise run dart:live                       # the public operations
INORBIT_API_KEY=... mise run dart:live   # and your account's
```
