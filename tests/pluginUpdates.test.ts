import { expect, test } from 'bun:test';
import { normalizePluginStore } from '../src/services/plugins';
import { getPluginUpdateStatus } from '../src/services/pluginUpdates';

const entry = { id: 'example', installed: true, source_id: 'original', installed_source_id: 'original', version: 'v1.0.0', installed_version: '1.0.0' };
const check = (plugins: unknown[], source_errors: unknown[] = []) => getPluginUpdateStatus(normalizePluginStore({ plugins, source_errors }), 'example');

test('reports updates only from the installed source, without confusing another registry', () => {
  expect(check([entry, { ...entry, source_id: 'other', update_available: true }])).toBe('updateCurrent');
  expect(check([{ ...entry, version: '2.0.0', update_available: true }])).toBe('updateFound');
});
test('does not claim current when a registry fails or installation provenance is uncertain', () => {
  expect(check([entry], [{ message: 'offline' }])).toBe('updateIncomplete');
  expect(check([])).toBe('updateMissing');
  expect(check([{ ...entry, installed_source_id: '' }])).toBe('updateReview');
  expect(check([{ ...entry, source_id: 'other' }])).toBe('updateReview');
  expect(check([entry, entry])).toBe('updateReview');
  expect(check([{ ...entry, installed_version: '' }])).toBe('updateReview');
  expect(check([{ ...entry, version: 'unknown' }])).toBe('updateReview');
});
