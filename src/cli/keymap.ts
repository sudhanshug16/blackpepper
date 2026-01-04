import type { KeyEvent } from "@opentui/core";

export type KeyChord = {
  key: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
};

const modifierAliases: Record<string, keyof Omit<KeyChord, "key">> = {
  ctrl: "ctrl",
  control: "ctrl",
  alt: "alt",
  option: "alt",
  shift: "shift",
  meta: "meta",
  cmd: "meta",
  super: "meta",
};

const keyAliases: Record<string, string> = {
  esc: "escape",
  escape: "escape",
  enter: "return",
  return: "return",
  space: "space",
  spacebar: "space",
  tab: "tab",
};

export function parseKeyChord(input: string): KeyChord | null {
  const trimmed = input.trim().toLowerCase();
  if (!trimmed) return null;

  const parts = trimmed
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length === 0) return null;

  const chord: KeyChord = {
    key: "",
    ctrl: false,
    alt: false,
    shift: false,
    meta: false,
  };

  for (const part of parts) {
    const modifier = modifierAliases[part];
    if (modifier) {
      chord[modifier] = true;
      continue;
    }

    if (chord.key) {
      return null;
    }

    chord.key = keyAliases[part] ?? part;
  }

  if (!chord.key) {
    return null;
  }

  return chord;
}

export function matchesChord(event: KeyEvent, chord: KeyChord): boolean {
  const name = event.name?.toLowerCase() ?? "";
  if (name !== chord.key) return false;

  return (
    event.ctrl === chord.ctrl &&
    event.meta === chord.meta &&
    event.shift === chord.shift &&
    event.option === chord.alt
  );
}
