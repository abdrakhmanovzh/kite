//! Explicit design-preview data. No Telegram account is connected.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Folder {
    All,
    Personal,
    Work,
}

pub struct Message {
    pub text: String,
    pub outgoing: bool,
    pub time: &'static str,
    pub photo: bool,
}

pub struct Chat {
    pub name: &'static str,
    pub summary: String,
    pub folder: Folder,
    pub unread: usize,
    pub color: u32,
    pub portrait: bool,
    pub messages: Vec<Message>,
    pub draft: String,
}

pub fn chats() -> Vec<Chat> {
    let mut chats: Vec<_> = [
        (
            "Maya Chen",
            "This place looks incredible!",
            Folder::Personal,
            2,
            0x486a80,
        ),
        (
            "Saved Messages",
            "Ideas for the weekend",
            Folder::Personal,
            0,
            0x3889ca,
        ),
        (
            "Alex Kim",
            "Sounds good, see you there!",
            Folder::Personal,
            1,
            0x6b617e,
        ),
        (
            "Weekend Crew",
            "Jordan: count me in!",
            Folder::Personal,
            3,
            0x4f7569,
        ),
        (
            "Family",
            "Mom: Have a great trip!",
            Folder::Personal,
            0,
            0x93724f,
        ),
        (
            "Design Team",
            "Chris: updated the files",
            Folder::Work,
            2,
            0x596f94,
        ),
        (
            "Sarah Park",
            "Let’s catch up next week",
            Folder::Personal,
            0,
            0x8b627b,
        ),
        ("Tokyo Trip", "You: Photo", Folder::Personal, 4, 0x587c80),
        ("Daniel Wu", "Got it, thanks!", Folder::Work, 0, 0x847452),
    ]
    .into_iter()
    .map(|(name, summary, folder, unread, color)| Chat {
        name,
        summary: summary.into(),
        folder,
        unread,
        color,
        portrait: name == "Maya Chen",
        messages: vec![Message {
            text: summary.into(),
            outgoing: false,
            time: "10:14",
            photo: false,
        }],
        draft: String::new(),
    })
    .collect();
    chats[0].messages = [
        ("Hey! Are you still up for the hiking trip this weekend?", false, "10:14", false),
        ("Absolutely! I’m so excited.\nThe weather looks perfect.", true, "10:17", false),
        ("I found a great trail that should be manageable but still have amazing views. Want to take a look?", false, "11:03", false),
        ("", false, "11:04", true),
        ("Wow this looks incredible!\nLet’s do it. What time should we meet?", true, "11:12", false),
        ("How about 7 AM at the trailhead?\nI’ll bring coffee and some snacks.", false, "11:18", false),
        ("Perfect! I’ll pack my gear tonight.\nCan’t wait!", true, "17:24", false),
    ].into_iter().map(|(text, outgoing, time, photo)| Message { text: text.into(), outgoing, time, photo }).collect();
    chats
}
