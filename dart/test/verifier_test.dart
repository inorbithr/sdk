import 'dart:convert';

import 'package:inorbit/inorbit.dart';
import 'package:inorbit/src/generated/contract.g.dart';
import 'package:test/test.dart';

// A small contract in the shape tool/gen_contract.dart writes.
final _contract = <String, Object?>{
  'api_version': '9.9.9',
  'operations': [
    {
      'id': 'ListThings',
      'method': 'GET',
      'path': '/v1/orgs/{org_id}/things',
      'answers': {
        '200': {r'$ref': '#/components/schemas/ListThingsResponse'},
      },
    },
    {
      'id': 'ListAssignments',
      'method': 'GET',
      'path': '/v1/orgs/assignments',
      'answers': {
        '200': {'type': 'object'},
      },
    },
    {
      'id': 'GetOrg',
      'method': 'GET',
      'path': '/v1/orgs/{org_id}',
      'answers': {
        '200': {'type': 'object'},
      },
    },
  ],
  'schemas': {
    'ListThingsResponse': {
      'type': 'object',
      'required': ['things', 'next_page_token'],
      'properties': {
        'things': {
          'type': 'array',
          'items': {r'$ref': '#/components/schemas/Thing'},
        },
        'next_page_token': {'type': 'string'},
      },
    },
    'Thing': {
      'type': 'object',
      'required': ['id', 'count', 'size', 'state', 'ratio', 'created_at'],
      'properties': {
        'id': {'type': 'string'},
        'count': {'type': 'integer', 'format': 'int32'},
        'size': {'type': 'integer', 'format': 'int64'},
        'state': {
          'type': 'string',
          'enum': ['up', 'down'],
        },
        'ratio': {'type': 'number', 'format': 'double'},
        'created_at': {'type': 'string'},
        'note': {
          'type': ['string', 'null'],
        },
        'tags': {
          'type': 'object',
          'additionalProperties': {'type': 'string'},
        },
        'owner': {
          'oneOf': [
            {'type': 'string'},
            {r'$ref': '#/components/schemas/Person'},
          ],
        },
      },
    },
    'Person': {
      'type': 'object',
      'required': ['name'],
      'properties': {
        'name': {'type': 'string'},
      },
    },
  },
};

Map<String, Object?> _thing([Map<String, Object?> change = const {}]) => {
  'id': 't-1',
  'count': 3,
  'size': '9007199254740993',
  'state': 'up',
  'ratio': 0.5,
  'created_at': '',
  ...change,
};

ContractCheck _verify(
  Object? body, {
  ContractMode mode = ContractMode.record,
}) => ContractVerifier.fromContract(
  _contract,
  mode: mode,
).verify(method: 'GET', path: '/v1/orgs/acme/things', status: 200, body: body);

void main() {
  test('an answer that keeps the contract has no mismatch', () {
    final check = _verify({
      'things': [
        _thing(),
        _thing({
          'note': null,
          'tags': {'a': 'b'},
        }),
      ],
      'next_page_token': '',
      'added_later': true, // a field the contract does not know is allowed
    });
    expect(check.covered, isTrue);
    expect(check.operation!.id, 'ListThings');
    expect(check.mismatches, isEmpty);
  });

  test('a missing required field is named by its pointer', () {
    final thing = _thing()..remove('state');
    final check = _verify({
      'things': [thing],
      'next_page_token': '',
    });
    expect(check.mismatches.single.field, '/things/*/state');
    expect(check.mismatches.single.expected, 'present');
    expect(check.mismatches.single.got, 'missing');
  });

  test(
    'a wrong kind, an unlisted value and a bad int64 are each found once',
    () {
      final check = _verify({
        'things': [
          _thing({'count': 'three', 'state': 'sideways', 'size': 'big'}),
          _thing({'count': 'three'}), // the same mismatch again: reported once
        ],
        'next_page_token': '',
      });
      final got = {
        for (final m in check.mismatches) m.field: '${m.expected}|${m.got}',
      };
      expect(got, {
        '/things/*/count': 'integer|string',
        '/things/*/state': 'one of the listed values|unlisted value',
        '/things/*/size': 'integer|string',
      });
    },
  );

  test('int64 as a number, doubles as integers and NaN are accepted', () {
    final check = _verify({
      'things': [
        _thing({'size': 12, 'ratio': 1, 'count': 2.0}),
        _thing({'ratio': 'NaN'}),
      ],
      'next_page_token': '',
    });
    expect(check.mismatches, isEmpty);
  });

  test('oneOf fits when any branch fits, and says so when none does', () {
    expect(
      _verify({
        'things': [
          _thing({'owner': 'ana'}),
        ],
        'next_page_token': '',
      }).mismatches,
      isEmpty,
    );
    expect(
      _verify({
        'things': [
          _thing({
            'owner': {'name': 'ana'},
          }),
        ],
        'next_page_token': '',
      }).mismatches,
      isEmpty,
    );
    final bad = _verify({
      'things': [
        _thing({'owner': 7}),
      ],
      'next_page_token': '',
    });
    expect(bad.mismatches.single.expected, 'one of 2 shapes');
    expect(bad.mismatches.single.got, 'integer');
  });

  test('mismatches carry no values: never ids, names or the concrete path', () {
    final check = _verify({
      'things': [
        _thing({'id': 42, 'state': 'secret-value'}),
      ],
      'next_page_token': '',
    });
    final text = [
      for (final m in check.mismatches) ...[...m.toJson().values, '$m'],
    ].join(' ');
    expect(text, isNot(contains('secret-value')));
    expect(text, isNot(contains('acme')));
    expect(text, isNot(contains('42')));
    expect(check.mismatches.first.path, '/v1/orgs/{org_id}/things');
    expect(check.mismatches.first.apiVersion, '9.9.9');
    expect(check.mismatches.first.sdkVersion, sdkVersion);
  });

  test('strict mode throws the mismatches', () {
    expect(
      () => _verify({
        'things': 'nope',
        'next_page_token': '',
      }, mode: ContractMode.strict),
      throwsA(
        isA<ContractViolation>().having(
          (v) => v.mismatches.single.field,
          'field',
          '/things',
        ),
      ),
    );
  });

  test('only arrays up to maxItems are walked', () {
    final verifier = ContractVerifier.fromContract(_contract);
    final many = [
      for (var i = 0; i < 60; i++) _thing(),
      _thing({'count': 'x'}),
    ];
    final check = verifier.verify(
      method: 'GET',
      path: '/v1/orgs/acme/things',
      status: 200,
      body: {'things': many, 'next_page_token': ''},
    );
    expect(check.mismatches, isEmpty, reason: 'item 61 is past maxItems (50)');
  });

  test('a call the contract does not know is not checked', () {
    final verifier = ContractVerifier.fromContract(_contract);
    expect(
      verifier
          .verify(method: 'GET', path: '/v1/elsewhere', status: 200, body: 1)
          .covered,
      isFalse,
    );
    expect(
      verifier
          .verify(
            method: 'POST',
            path: '/v1/orgs/acme/things',
            status: 200,
            body: 1,
          )
          .covered,
      isFalse,
    );
  });

  test('a literal path wins over a template', () {
    final verifier = ContractVerifier.fromContract(_contract);
    expect(
      verifier.operationFor('get', '/v1/orgs/assignments')!.id,
      'ListAssignments',
    );
    expect(verifier.operationFor('GET', '/v1/orgs/acme')!.id, 'GetOrg');
    expect(verifier.operationFor('GET', '/v1/orgs/acme/things/extra'), isNull);
  });

  group('the bundled contract', () {
    final verifier = ContractVerifier.bundled();

    test('holds every operation of spec/openapi.json', () {
      expect(verifier.operations.length, greaterThanOrEqualTo(100));
      expect(verifier.apiVersion, isNotEmpty);
      expect(verifier.operationFor('GET', '/v1/me')?.id, isNotNull);
      expect(
        verifier.operationFor('GET', '/v1/accounts/orgs/o-1/monitors')?.path,
        '/v1/accounts/orgs/{org_id}/monitors',
      );
    });

    test('every reference names a schema the contract holds', () {
      final contract = jsonDecode(contractJson) as Map<String, Object?>;
      final schemas = contract['schemas']! as Map<String, Object?>;
      final dangling = <String>{};
      void walk(Object? node) {
        if (node is Map) {
          final ref = node[r'$ref'];
          if (ref is String && !schemas.containsKey(ref.split('/').last)) {
            dangling.add(ref);
          }
          node.values.forEach(walk);
        } else if (node is List) {
          node.forEach(walk);
        }
      }

      walk(contract);
      expect(dangling, isEmpty);
    });
  });
}
