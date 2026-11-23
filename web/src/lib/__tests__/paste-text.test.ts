import { describe, it, expect } from "vitest";

import { normalizePastedText } from "../paste-text";

describe("normalizePastedText", () => {
  it("unifies line endings", () => {
    // Lexical only tokenizes \n and \r\n: a bare \r (spreadsheets, PDFs, older
    // Mac apps) would otherwise land in the message as an invisible control
    // character and the lines would run together.
    expect(normalizePastedText("a\r\nb\rc\u2028d\u2029e\u0085f")).toBe(
      "a\nb\nc\nd\ne\nf",
    );
  });

  it("turns lookalike spaces into real spaces", () => {
    expect(normalizePastedText("Hello\u00a0world\u2009again\u3000here")).toBe(
      "Hello world again here",
    );
  });

  it("strips invisible characters", () => {
    expect(normalizePastedText("\ufeff/dep\u200bloy soft\u00adhyphen")).toBe(
      "/deploy softhyphen",
    );
    expect(normalizePastedText("word\u2060joiner")).toBe("wordjoiner");
  });

  // ZWNJ and ZWJ sit in the same Unicode block as the zero-width space above,
  // and stripping the block wholesale silently rewrote the user's words.
  it("keeps the zero-width joiners, which carry meaning", () => {
    // ZWNJ is orthographic: \u0645\u064a\u200c\u0631\u0648\u0645 ("I go") is not \u0645\u064a\u0631\u0648\u0645.
    expect(normalizePastedText("\u0645\u06cc\u200c\u0631\u0648\u0645")).toBe("\u0645\u06cc\u200c\u0631\u0648\u0645");
    // ZWJ is what makes one emoji out of several code points.
    expect(normalizePastedText("\ud83d\udc69\u200d\ud83d\udcbb \ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67")).toBe(
      "\ud83d\udc69\u200d\ud83d\udcbb \ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67",
    );
    // A joiner still survives next to an invisible that does get stripped.
    expect(normalizePastedText("\ufeff\u0645\u06cc\u200c\u0631\u0648\u0645")).toBe("\u0645\u06cc\u200c\u0631\u0648\u0645");
  });

  it("drops stray control characters but keeps tabs and newlines", () => {
    expect(normalizePastedText("col\tone\nrow\u0007two\u0000")).toBe(
      "col\tone\nrowtwo",
    );
  });

  it("leaves visible punctuation alone", () => {
    const text = "“Smart quotes” — em dashes … and emoji 🎉";
    expect(normalizePastedText(text)).toBe(text);
  });

  it("is a no-op for text that is already clean", () => {
    expect(normalizePastedText("plain text\nsecond line")).toBe(
      "plain text\nsecond line",
    );
  });
});
