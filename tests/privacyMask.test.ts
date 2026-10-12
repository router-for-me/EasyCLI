import { expect, test } from 'bun:test';
import { maskEmail, maskIdentifier, maskSensitiveText } from '../src/services/privacyMask';
import { authFileIdentities } from '../src/services/authFiles';

test('masks emails while keeping enough shape to tell accounts apart', () => {
  expect(maskEmail('alice.smith@example.com')).toBe('al***@e***.com');
  expect(maskEmail('bob@corp.example.co')).toBe('b***@c***.co');
});

test('masks opaque identifiers without revealing their middle', () => {
  expect(maskIdentifier('my-gcp-project-123456')).toBe('my***56');
  expect(maskIdentifier('proj-12')).toBe('p***2');
  expect(maskIdentifier('abcd')).toBe('***');
  expect(maskIdentifier('svc@my-project.iam.gserviceaccount.com')).toBe('s***@m***.com');
  expect(maskIdentifier('')).toBe('');
});

test('masks every email in free text, including backend errors', () => {
  expect(maskSensitiveText('token for alice@example.com and bob@test.io expired'))
    .toBe('token for al***@e***.com and b***@t***.io expired');
  expect(maskSensitiveText('Codex · 3f9a2c')).toBe('Codex · 3f9a2c');
  expect(maskSensitiveText('')).toBe('');
});

test('masks proxy URL credentials', () => {
  expect(maskSensitiveText('socks5://example:example@127.0.0.1:1080')).toBe('socks5://***@127.0.0.1:1080');
  expect(maskSensitiveText('http://proxy.local:8080')).toBe('http://proxy.local:8080');
});

test('masks known identities inside credential file names, including sanitized forms', () => {
  const identities = ['alice@example.com', 'my-gcp-project-123456'];
  expect(maskSensitiveText('codex-alice@example.com-plus.json', identities))
    .toBe('codex-al***@e***.com-plus.json');
  expect(maskSensitiveText('antigravity-alice_example_com.json', identities))
    .toBe('antigravity-al***@e***.com.json');
  expect(maskSensitiveText('alice@example.com-my-gcp-project-123456.json', identities))
    .toBe('al***@e***.com-my***56.json');
  // Without the record's identity, the generic pattern still hides the address.
  expect(maskSensitiveText('codex-alice@example.com.json')).not.toContain('alice');
});

test('collects the account identifiers of a credential record', () => {
  expect(authFileIdentities({ name: 'x.json', email: ' alice@example.com ', project_id: 'proj-1', label: 'Work' }))
    .toEqual(['alice@example.com', 'proj-1']);
  expect(authFileIdentities({ name: 'x.json' })).toEqual([]);
});
