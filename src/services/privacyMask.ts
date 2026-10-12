// Privacy mode masks account identities at render time only. Callers keep the
// raw values for API requests, filters, search and copy actions.

const MASK = '***';
const EMAIL_PATTERN = /[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}/g;
// Login credentials embedded in proxy or upstream URLs.
const URL_USERINFO_PATTERN = /([A-Za-z][A-Za-z0-9+.-]*:\/\/)[^\s/@]+@/g;

const keepPrefix = (value: string, visible: number) => value.slice(0, Math.min(visible, value.length));

export function maskEmail(email: string): string {
  const at = email.lastIndexOf('@');
  if (at <= 0) return maskIdentifier(email);
  const local = email.slice(0, at);
  const domain = email.slice(at + 1);
  const dot = domain.lastIndexOf('.');
  const host = dot > 0 ? domain.slice(0, dot) : domain;
  const suffix = dot > 0 ? domain.slice(dot) : '';
  return `${keepPrefix(local, local.length > 4 ? 2 : 1)}${MASK}@${keepPrefix(host, 1)}${MASK}${suffix}`;
}

/** Masks an opaque account identifier such as a GCP project or account ID. */
export function maskIdentifier(value: string): string {
  const text = value.trim();
  if (!text) return value;
  if (text.includes('@')) return maskEmail(text);
  if (text.length <= 4) return MASK;
  const visible = text.length > 10 ? 2 : 1;
  return `${text.slice(0, visible)}${MASK}${text.slice(-visible)}`;
}

// Credential file names often embed the account with `@` and `.` replaced,
// e.g. antigravity-alice_example_com.json, so the email pattern alone misses them.
const identityVariants = (identity: string) => {
  const value = identity.trim();
  if (value.length < 3) return [];
  const variants = new Set([value]);
  if (value.includes('@')) {
    variants.add(value.replace(/[@.]/g, '_'));
    variants.add(value.replace(/[@.]/g, '-'));
    variants.add(value.replace('@', '_'));
  }
  return [...variants].sort((left, right) => right.length - left.length);
};

const escapeRegExp = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * Masks emails and URL credentials anywhere in free text. Known identities of
 * the record being rendered are masked first, including their filename forms.
 */
export function maskSensitiveText(text: string, identities: readonly string[] = []): string {
  if (!text) return text;
  let masked = text;
  for (const identity of identities) {
    for (const variant of identityVariants(identity)) {
      masked = masked.replace(new RegExp(escapeRegExp(variant), 'gi'), maskIdentifier(identity));
    }
  }
  return masked
    .replace(URL_USERINFO_PATTERN, `$1${MASK}@`)
    .replace(EMAIL_PATTERN, (email) => maskEmail(email));
}
