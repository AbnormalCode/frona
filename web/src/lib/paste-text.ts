/**
 * Clipboard text hygiene for the chat composer.
 *
 * The composer's paste handler inserts whatever the clipboard holds, so these
 * survive into the message: CR-only line endings, U+2028/U+2029 and NEL (common
 * in text copied from PDFs and spreadsheets) as literal control characters,
 * and non-breaking spaces and zero-width characters (from Word and Google
 * Docs) that read as ordinary spaces but break the `/command` and `@agent`
 * trigger match, which splits on real whitespace, and travel on to the agent
 * as invisible junk.
 */

/** CRLF, CR, NEL, LINE SEPARATOR, PARAGRAPH SEPARATOR → a plain newline. */
const LINE_SEPARATORS = /\r\n|[\r\u0085\u2028\u2029]/g;

/** Spaces that aren't U+0020: NBSP, the en/em quad family, narrow + ideographic. */
const SPACE_LOOKALIKES = /[\u00a0\u1680\u2000-\u200a\u202f\u205f\u3000]/g;

/**
 * Zero-width and soft-hyphen characters: invisible, and they split words.
 *
 * The two joiners in this block are deliberately NOT here. U+200C (ZWNJ) is
 * orthographic in Persian, Arabic and the Indic scripts \u2014 `\u0645\u06cc\u200c\u0631\u0648\u0645` and `\u0645\u06cc\u0631\u0648\u0645`
 * are different words \u2014 and U+200D (ZWJ) is what holds an emoji sequence
 * together, so stripping it turns `\ud83d\udc69\u200d\ud83d\udcbb` into two separate emoji. They are the
 * user's text, not formatting noise, and the same rule that keeps smart quotes
 * keeps them. What remains is zero-width *space*, the soft hyphen, the word
 * joiner and the BOM: separators with no meaning inside a message.
 */
const INVISIBLES = /[\u00ad\u200b\u2060\ufeff]/g;

/** Control characters with no meaning in a message. Tabs and newlines are kept. */
const CONTROL_CHARS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g;

/**
 * Make pasted text safe to insert: one newline convention, real spaces, and no
 * invisible characters. Deliberately leaves the visible content alone — smart
 * quotes, em dashes and emoji are the user's text, not formatting noise.
 */
export function normalizePastedText(text: string): string {
  return text
    .replace(LINE_SEPARATORS, "\n")
    .replace(SPACE_LOOKALIKES, " ")
    .replace(INVISIBLES, "")
    .replace(CONTROL_CHARS, "");
}
