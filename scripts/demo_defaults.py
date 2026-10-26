#!/usr/bin/env python3
"""Generates src/demo/defaults.json: a complete default JSON object for each
TDLib type the demo seeds, read from tdlib-rs's generated bindings.

TDLib types have no serde defaults, so seed data patches these templates.
Usage: python3 scripts/demo_defaults.py target/debug/build/tdlib-rs-*/out/generated.rs
"""
import json
import re
import sys

source = open(sys.argv[1]).read()


def block(kind, name):
    match = re.search(r"\n    pub %s %s \{\n(.*?)\n    \}\n" % (kind, re.escape(name)), source, re.S)
    return match.group(1) if match else None


def struct_fields(name):
    body = block("struct", name)
    if body is None:
        raise KeyError(name)
    fields, display = [], False
    for line in body.splitlines():
        line = line.strip()
        if line.startswith("#[serde_as(as = \"DisplayFromStr\")]"):
            display = True
        match = re.match(r"pub (r#)?(\w+): (.+),$", line)
        if match:
            fields.append((match.group(2), match.group(3), display))
            display = False
    return fields


def enum_variants(name):
    body = block("enum", name)
    variants, rename = [], None
    for line in body.splitlines():
        line = line.strip()
        match = re.search(r'rename\(serialize = "(\w+)"', line)
        if match:
            rename = match.group(1)
            continue
        match = re.match(r"(\w+)(\((crate::types::\w+)\))?,$", line)
        if match and rename:
            variants.append((match.group(1), rename, match.group(3)))
            rename = None
    return variants


def default(ty, display=False, depth=0):
    ty = ty.strip()
    if ty.startswith("Option<"):
        return None
    if ty.startswith("Vec<"):
        return []
    if ty in ("i32", "i16", "i8", "u8"):
        return 0
    if ty == "i64":
        return "0" if display else 0
    if ty == "f64" or ty == "f32":
        return 0.0
    if ty == "bool":
        return False
    if ty == "String":
        return ""
    if ty.startswith("crate::types::"):
        return struct_default(ty.split("::")[-1], depth)
    if ty.startswith("crate::enums::"):
        return enum_default(ty.split("::")[-1], None, depth)
    raise ValueError(ty)


def struct_default(name, depth=0):
    if depth > 12:
        raise RecursionError(name)
    out = {}
    for field, ty, display in struct_fields(name):
        value = default(ty, display, depth + 1)
        if value is not None:
            out[field] = value
    return out


def enum_default(name, variant=None, depth=0):
    variants = enum_variants(name)
    choices = [v for v in variants if variant is None or v[0] == variant]
    # Prefer unit variants, then the first that can be built.
    for _, rename, inner in sorted(choices, key=lambda v: v[2] is not None):
        try:
            value = struct_default(inner.split("::")[-1], depth + 1) if inner else {}
        except (KeyError, RecursionError, ValueError):
            continue
        value["@type"] = rename
        return value
    raise ValueError(name)


def variant(enum, name):
    return enum_default(enum, name)


out = {
    "Chat": struct_default("Chat"),
    "Message": struct_default("Message"),
    "User": struct_default("User"),
    "File": struct_default("File"),
    "Sticker": struct_default("Sticker"),
    "PhotoSize": struct_default("PhotoSize"),
    "ChatPosition": struct_default("ChatPosition"),
    "ChatFolderInfo": struct_default("ChatFolderInfo"),
    "Supergroup": struct_default("Supergroup"),
    "BasicGroup": struct_default("BasicGroup"),
    "MessageReaction": struct_default("MessageReaction"),
    "MessageInteractionInfo": struct_default("MessageInteractionInfo"),
    "PollOption": struct_default("PollOption"),
    "Poll": struct_default("Poll"),
    "ProfilePhoto": struct_default("ProfilePhoto"),
    "ChatPhotoInfo": struct_default("ChatPhotoInfo"),
    "MessageReactions": struct_default("MessageReactions"),
    "MessageForwardInfo": struct_default("MessageForwardInfo"),
    "VoiceNote": struct_default("VoiceNote"),
    "VideoNote": struct_default("VideoNote"),
    "Document": struct_default("Document"),
    "Photo": struct_default("Photo"),
    "FormattedText": struct_default("FormattedText"),
    "TextEntity": struct_default("TextEntity"),
    "variants": {
        "ChatType": {n: variant("ChatType", n) for n in ["Private", "BasicGroup", "Supergroup"]},
        "UserStatus": {n: variant("UserStatus", n) for n in ["Online", "Offline", "Recently"]},
        "MessageSender": {n: variant("MessageSender", n) for n in ["User", "Chat"]},
        "MessageReplyTo": {"Message": variant("MessageReplyTo", "Message")},
        "MessageOrigin": {"User": variant("MessageOrigin", "User")},
        "ChatList": {n: variant("ChatList", n) for n in ["Main", "Archive", "Folder"]},
        "ReactionType": {"Emoji": variant("ReactionType", "Emoji")},
        "StickerFormat": {n: variant("StickerFormat", n) for n in ["Webp", "Tgs", "Webm"]},
        "TextEntityType": {
            n: variant("TextEntityType", n)
            for n in ["Bold", "Italic", "Code", "Strikethrough", "Url", "TextUrl", "Pre", "Spoiler"]
        },
        "PollType": {"Regular": variant("PollType", "Regular")},
    },
    "content": {
        name: variant("MessageContent", name)
        for name in [
            "MessageText", "MessagePhoto", "MessageSticker", "MessageVoiceNote",
            "MessageVideoNote", "MessageDocument", "MessageAudio", "MessagePoll",
            "MessageChatAddMembers", "MessagePinMessage",
        ]
    },
}
json.dump(out, sys.stdout, indent=1, ensure_ascii=False)
