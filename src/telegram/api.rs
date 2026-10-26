//! The part of TDLib's API that FinchGram uses, as serde types, written against td_api.tl of
//! [`super::TDLIB_VERSION`]. Every TDLib object names its type in "@type".
//!
//! Fields that are not listed here are ignored, so a newer TDLib may add fields freely. One that
//! renames or removes a field we read fails to deserialize: the update is logged and dropped
//! (process.rs). When TDLib is upgraded, compare its td_api.tl with the types here.
//!
//! Requests are written with serde_json's json! macro where they are sent: they are TDLib's own
//! objects, and td_api.tl is their documentation.
//!
//! In TDLib's JSON, int53 values are numbers and int64 values are strings (see [`int64`]); an
//! object that is null may be missing, hence the `#[serde(default)]` on optional fields.

use serde::Deserialize;

/// Something changed. Only the updates FinchGram follows are listed; every other kind is `Other`
/// and is dropped as soon as it is read.
#[derive(Debug, Deserialize)]
#[serde(tag = "@type")]
pub enum Update {
    #[serde(rename = "updateAuthorizationState")]
    AuthorizationState { authorization_state: AuthorizationState },
    #[serde(rename = "updateConnectionState")]
    ConnectionState { state: ConnectionState },
    #[serde(rename = "updateOption")]
    Option { name: String, value: OptionValue },

    #[serde(rename = "updateUser")]
    User { user: User },
    #[serde(rename = "updateUserStatus")]
    UserStatus { user_id: i64, status: UserStatus },
    #[serde(rename = "updateUserFullInfo")]
    UserFullInfo { user_id: i64, user_full_info: UserFullInfo },
    #[serde(rename = "updateBasicGroup")]
    BasicGroup { basic_group: BasicGroup },
    #[serde(rename = "updateSupergroup")]
    Supergroup { supergroup: Supergroup },
    #[serde(rename = "updateSupergroupFullInfo")]
    SupergroupFullInfo { supergroup_id: i64, supergroup_full_info: SupergroupFullInfo },

    #[serde(rename = "updateNewChat")]
    NewChat { chat: Box<Chat> },
    #[serde(rename = "updateChatTitle")]
    ChatTitle { chat_id: i64, title: String },
    #[serde(rename = "updateChatPhoto")]
    ChatPhoto {
        chat_id: i64,
        #[serde(default)]
        photo: Option<ChatPhoto>,
    },
    #[serde(rename = "updateChatPermissions")]
    ChatPermissions { chat_id: i64, permissions: ChatPermissions },
    #[serde(rename = "updateChatLastMessage")]
    ChatLastMessage {
        chat_id: i64,
        #[serde(default)]
        last_message: Option<Box<Message>>,
        positions: Vec<ChatPosition>,
    },
    #[serde(rename = "updateChatPosition")]
    ChatPosition { chat_id: i64, position: ChatPosition },
    #[serde(rename = "updateChatDraftMessage")]
    ChatDraftMessage {
        chat_id: i64,
        #[serde(default)]
        draft_message: Option<DraftMessage>,
        positions: Vec<ChatPosition>,
    },
    #[serde(rename = "updateMessageInteractionInfo")]
    MessageInteractionInfo {
        chat_id: i64,
        message_id: i64,
        #[serde(default)]
        interaction_info: Option<MessageInteractionInfo>,
    },
    #[serde(rename = "updateChatAvailableReactions")]
    ChatAvailableReactions { chat_id: i64, available_reactions: ChatAvailableReactions },
    #[serde(rename = "updateDefaultReactionType")]
    DefaultReactionType { reaction_type: ReactionType },
    #[serde(rename = "updateForumTopicInfo")]
    ForumTopicInfo { info: ForumTopicInfo },
    #[serde(rename = "updateForumTopic")]
    ForumTopic {
        chat_id: i64,
        forum_topic_id: i32,
        is_pinned: bool,
        last_read_inbox_message_id: i64,
        #[serde(default)]
        unread_mention_count: i32,
    },
    #[serde(rename = "updateChatReadInbox")]
    ChatReadInbox { chat_id: i64, last_read_inbox_message_id: i64, unread_count: i32 },
    #[serde(rename = "updateChatReadOutbox")]
    ChatReadOutbox { chat_id: i64, last_read_outbox_message_id: i64 },
    #[serde(rename = "updateChatUnreadMentionCount")]
    ChatUnreadMentionCount { chat_id: i64, unread_mention_count: i32 },
    #[serde(rename = "updateMessageMentionRead")]
    MessageMentionRead { chat_id: i64, unread_mention_count: i32 },
    #[serde(rename = "updateChatNotificationSettings")]
    ChatNotificationSettings { chat_id: i64, notification_settings: ChatNotificationSettings },
    #[serde(rename = "updateScopeNotificationSettings")]
    ScopeNotificationSettings { scope: NotificationSettingsScope, notification_settings: ScopeNotificationSettings },
    #[serde(rename = "updateChatIsMarkedAsUnread")]
    ChatIsMarkedAsUnread { chat_id: i64, is_marked_as_unread: bool },
    #[serde(rename = "updateChatBlockList")]
    ChatBlockList {
        chat_id: i64,
        #[serde(default)]
        block_list: Option<BlockList>,
    },
    #[serde(rename = "updateChatFolders")]
    ChatFolders { chat_folders: Vec<ChatFolderInfo> },
    #[serde(rename = "updateChatOnlineMemberCount")]
    ChatOnlineMemberCount { chat_id: i64, online_member_count: i32 },
    #[serde(rename = "updateChatAction")]
    ChatAction { chat_id: i64, sender_id: MessageSender, action: ChatAction },

    #[serde(rename = "updateNewMessage")]
    NewMessage { message: Box<Message> },
    #[serde(rename = "updateMessageSendSucceeded")]
    MessageSendSucceeded { message: Box<Message>, old_message_id: i64 },
    #[serde(rename = "updateMessageSendFailed")]
    MessageSendFailed { message: Box<Message>, old_message_id: i64 },
    #[serde(rename = "updateMessageContent")]
    MessageContent { chat_id: i64, message_id: i64, new_content: MessageContent },
    /// The content was opened: a voice message listened to, a round video message watched.
    #[serde(rename = "updateMessageContentOpened")]
    MessageContentOpened { chat_id: i64, message_id: i64 },
    /// Edited: its new buttons come with it (none when they went).
    #[serde(rename = "updateMessageEdited")]
    MessageEdited {
        chat_id: i64,
        message_id: i64,
        edit_date: i32,
        #[serde(default)]
        reply_markup: Option<ReplyMarkup>,
    },
    #[serde(rename = "updateMessageIsPinned")]
    MessageIsPinned { chat_id: i64, message_id: i64, is_pinned: bool },
    /// The message whose keyboard the chat shows under the composer (a bot's), or none.
    #[serde(rename = "updateChatReplyMarkup")]
    ChatReplyMarkup {
        chat_id: i64,
        /// The message whose keyboard the chat now shows; 0 for none.
        reply_markup_message_id: i64,
    },
    #[serde(rename = "updateChatHasProtectedContent")]
    ChatHasProtectedContent { chat_id: i64, has_protected_content: bool },
    #[serde(rename = "updateDeleteMessages")]
    DeleteMessages { chat_id: i64, message_ids: Vec<i64>, is_permanent: bool },

    /// A download went on (files.rs).
    #[serde(rename = "updateFile")]
    File { file: File },

    /// Notifications (notifications.rs): TDLib added some to a chat's group of them, or took some
    /// back. `notification_settings_chat_id` is 0 when the added ones are not new but show again
    /// (a group that had made room for others comes back); `notification_sound_id` is 0 when they
    /// make no sound, -1 for the default sound.
    #[serde(rename = "updateNotificationGroup")]
    NotificationGroup {
        notification_group_id: i32,
        chat_id: i64,
        notification_settings_chat_id: i64,
        #[serde(with = "int64")]
        notification_sound_id: i64,
        added_notifications: Vec<Notification>,
        removed_notification_ids: Vec<i32>,
    },
    /// The notifications still shown from before TDLib started, once, before any other.
    #[serde(rename = "updateActiveNotifications")]
    ActiveNotifications { groups: Vec<NotificationGroup> },
    /// Unread messages in a chat list: all of them, and those in chats that are not muted.
    #[serde(rename = "updateUnreadMessageCount")]
    UnreadMessageCount { chat_list: ChatList, unread_count: i32, unread_unmuted_count: i32 },

    #[serde(other)]
    Other,
}

/// Where logging in is.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum AuthorizationState {
    #[serde(rename = "authorizationStateWaitTdlibParameters")]
    WaitTdlibParameters,
    #[serde(rename = "authorizationStateWaitPhoneNumber")]
    WaitPhoneNumber,
    #[serde(rename = "authorizationStateWaitPremiumPurchase")]
    WaitPremiumPurchase,
    #[serde(rename = "authorizationStateWaitEmailAddress")]
    WaitEmailAddress,
    #[serde(rename = "authorizationStateWaitEmailCode")]
    WaitEmailCode { code_info: EmailAddressAuthenticationCodeInfo },
    #[serde(rename = "authorizationStateWaitCode")]
    WaitCode { code_info: AuthenticationCodeInfo },
    /// Logging in with a QR code: `link` is what the code shows, confirmed on a device that is
    /// already logged in.
    #[serde(rename = "authorizationStateWaitOtherDeviceConfirmation")]
    WaitOtherDeviceConfirmation { link: String },
    /// The phone number has no account yet: signing up (registerUser) makes one.
    #[serde(rename = "authorizationStateWaitRegistration")]
    WaitRegistration,
    /// The account has a two-step verification password. `recovery_email_address_pattern` is
    /// where a recovery code went, once one was asked for.
    #[serde(rename = "authorizationStateWaitPassword")]
    WaitPassword {
        password_hint: String,
        #[serde(default)]
        has_recovery_email_address: bool,
        #[serde(default)]
        recovery_email_address_pattern: String,
    },
    #[serde(rename = "authorizationStateReady")]
    Ready,
    #[serde(rename = "authorizationStateLoggingOut")]
    LoggingOut,
    #[serde(rename = "authorizationStateClosing")]
    Closing,
    /// TDLib has closed. finchgram-tdlib ends right after saying so.
    #[serde(rename = "authorizationStateClosed")]
    Closed,
}

/// Where a login code went, and how long it is.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthenticationCodeInfo {
    pub phone_number: String,
    #[serde(rename = "type")]
    pub kind: AuthenticationCodeType,
    #[serde(default)]
    pub next_type: Option<AuthenticationCodeType>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum AuthenticationCodeType {
    #[serde(rename = "authenticationCodeTypeTelegramMessage")]
    TelegramMessage { length: i32 },
    #[serde(rename = "authenticationCodeTypeSms")]
    Sms { length: i32 },
    #[serde(rename = "authenticationCodeTypeCall")]
    Call { length: i32 },
    #[serde(rename = "authenticationCodeTypeFlashCall")]
    FlashCall,
    #[serde(rename = "authenticationCodeTypeMissedCall")]
    MissedCall { length: i32 },
    #[serde(rename = "authenticationCodeTypeFragment")]
    Fragment { length: i32 },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EmailAddressAuthenticationCodeInfo {
    pub email_address_pattern: String,
    pub length: i32,
}

/// The two-step verification password of the account logged in (getPasswordState).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PasswordState {
    pub has_password: bool,
    #[serde(default)]
    pub password_hint: String,
    pub has_recovery_email_address: bool,
    /// A new recovery email address waits for the code sent to it.
    #[serde(default)]
    pub recovery_email_address_code_info: Option<EmailAddressAuthenticationCodeInfo>,
    /// When a reset asked for without the password can be completed; 0 when none is pending.
    #[serde(default)]
    pub pending_reset_date: i32,
}

/// The recovery email address, which TDLib only gives for the password (getRecoveryEmailAddress).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RecoveryEmailAddress {
    pub recovery_email_address: String,
}

/// The answer to resetPassword: a password forgotten without a recovery email address is removed
/// only after a wait.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ResetPasswordResult {
    #[serde(rename = "resetPasswordResultOk")]
    Ok,
    #[serde(rename = "resetPasswordResultPending")]
    Pending { pending_reset_date: i32 },
    #[serde(rename = "resetPasswordResultDeclined")]
    Declined { retry_date: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ConnectionState {
    #[serde(rename = "connectionStateWaitingForNetwork")]
    WaitingForNetwork,
    #[serde(rename = "connectionStateConnectingToProxy")]
    ConnectingToProxy,
    #[serde(rename = "connectionStateConnecting")]
    Connecting,
    #[serde(rename = "connectionStateUpdating")]
    Updating,
    #[serde(rename = "connectionStateReady")]
    Ready,
}

/// TDLib's answer when a request failed.
#[derive(Debug, Clone, Deserialize)]
pub struct TdError {
    pub code: i32,
    pub message: String,
}

/// The value of an option (getOption, updateOption).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum OptionValue {
    #[serde(rename = "optionValueBoolean")]
    Boolean { value: bool },
    #[serde(rename = "optionValueEmpty")]
    Empty,
    #[serde(rename = "optionValueInteger")]
    Integer {
        #[serde(with = "int64")]
        value: i64,
    },
    #[serde(rename = "optionValueString")]
    String { value: String },
}

// ---- users and groups -------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    #[serde(default)]
    pub usernames: Option<Usernames>,
    pub phone_number: String,
    #[serde(default)]
    pub profile_photo: Option<ChatPhoto>,
    pub status: UserStatus,
    /// Among the account's contacts.
    #[serde(default)]
    pub is_contact: bool,
    #[serde(default)]
    pub verification_status: Option<VerificationStatus>,
    #[serde(rename = "type")]
    pub kind: UserType,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Usernames {
    pub active_usernames: Vec<String>,
    pub editable_username: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VerificationStatus {
    pub is_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum UserType {
    #[serde(rename = "userTypeRegular")]
    Regular,
    #[serde(rename = "userTypeBot")]
    Bot,
    #[serde(rename = "userTypeDeleted")]
    Deleted,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum UserStatus {
    #[serde(rename = "userStatusEmpty")]
    Empty,
    #[serde(rename = "userStatusOnline")]
    Online { expires: i32 },
    #[serde(rename = "userStatusOffline")]
    Offline { was_online: i32 },
    #[serde(rename = "userStatusRecently")]
    Recently,
    #[serde(rename = "userStatusLastWeek")]
    LastWeek,
    #[serde(rename = "userStatusLastMonth")]
    LastMonth,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserFullInfo {
    #[serde(default)]
    pub bio: Option<FormattedText>,
    /// For a bot: its menu button and its commands.
    #[serde(default)]
    pub bot_info: Option<BotInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BasicGroup {
    pub id: i64,
    pub member_count: i32,
    pub status: ChatMemberStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Supergroup {
    pub id: i64,
    #[serde(default)]
    pub usernames: Option<Usernames>,
    pub status: ChatMemberStatus,
    pub member_count: i32,
    #[serde(default)]
    pub verification_status: Option<VerificationStatus>,
    /// A group with topics: it opens on its list of topics (topics.rs).
    #[serde(default)]
    pub is_forum: bool,
    /// Those who come by an invite link wait for an administrator's word.
    #[serde(default)]
    pub join_by_request: bool,
    /// A channel whose posts carry their author's name (manage.rs).
    #[serde(default)]
    pub sign_messages: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SupergroupFullInfo {
    pub member_count: i32,
    #[serde(default)]
    pub administrator_count: i32,
    #[serde(default)]
    pub description: String,
    /// Slow mode: how many seconds a member waits between messages (0: off), and how long we
    /// still wait after our last one (admin.rs).
    #[serde(default)]
    pub slow_mode_delay: i32,
    #[serde(default)]
    pub slow_mode_delay_expires_in: f64,
    #[serde(default)]
    pub has_hidden_members: bool,
    #[serde(default)]
    pub can_hide_members: bool,
    #[serde(default)]
    pub can_toggle_aggressive_anti_spam: bool,
    #[serde(default)]
    pub is_all_history_available: bool,
    #[serde(default)]
    pub has_aggressive_anti_spam_enabled: bool,
    /// The chat's primary invite link, for an administrator who may invite; none otherwise.
    #[serde(default)]
    pub invite_link: Option<ChatInviteLink>,
    /// The commands of the bots in the group.
    #[serde(default)]
    pub bot_commands: Vec<BotCommands>,
    /// A channel's discussion group, or a group's channel; 0 for none (manage.rs).
    #[serde(default)]
    pub linked_chat_id: i64,
    /// The owner may give the chat a username (make it public).
    #[serde(default)]
    pub can_set_username: bool,
}

/// An invite link of a chat (chatInviteLink): the primary one, or one made with a name, an expiry
/// and a limit (manage.rs).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ChatInviteLink {
    pub invite_link: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub creator_user_id: i64,
    #[serde(default)]
    pub date: i32,
    /// When it stops working, as seconds since the epoch; 0 for never.
    #[serde(default)]
    pub expiration_date: i32,
    /// How many may join by it; 0 for no limit.
    #[serde(default)]
    pub member_limit: i32,
    #[serde(default)]
    pub member_count: i32,
    #[serde(default)]
    pub creates_join_request: bool,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub is_revoked: bool,
}

/// Invite links found (chatInviteLinks).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChatInviteLinks {
    #[serde(default)]
    pub invite_links: Vec<ChatInviteLink>,
}

/// A member of a group or channel (chatMember), as getSupergroupMembers and a basic group's full
/// info list them (chat_info.rs, manage.rs).
#[derive(Debug, Clone, Deserialize)]
pub struct ChatMember {
    pub member_id: MessageSender,
    /// Who brought them in; for an administrator, who made them one. 0 when unknown.
    #[serde(default)]
    pub inviter_user_id: i64,
    pub status: ChatMemberStatus,
}

/// Members found (chatMembers): how many there are in all, and those asked for.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChatMembers {
    pub total_count: i32,
    #[serde(default)]
    pub members: Vec<ChatMember>,
}

/// What we are in a group or channel: who may write where.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatMemberStatus {
    #[serde(rename = "chatMemberStatusCreator")]
    Creator,
    #[serde(rename = "chatMemberStatusAdministrator")]
    Administrator {
        /// Shown beside their name in the group; "" for the plain "admin".
        #[serde(default)]
        custom_title: String,
        /// We may change their rights or dismiss them.
        #[serde(default)]
        can_be_edited: bool,
        rights: AdministratorRights,
    },
    #[serde(rename = "chatMemberStatusMember")]
    Member,
    #[serde(rename = "chatMemberStatusRestricted")]
    Restricted { is_member: bool, permissions: ChatPermissions },
    #[serde(rename = "chatMemberStatusLeft")]
    Left,
    #[serde(rename = "chatMemberStatusBanned")]
    Banned,
}

/// What an administrator may do (chatAdministratorRights). Every field, so that a status can be
/// sent back changed (setChatMemberStatus, manage.rs).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct AdministratorRights {
    #[serde(default)]
    pub can_manage_chat: bool,
    #[serde(default)]
    pub can_change_info: bool,
    #[serde(default)]
    pub can_post_messages: bool,
    #[serde(default)]
    pub can_edit_messages: bool,
    #[serde(default)]
    pub can_delete_messages: bool,
    #[serde(default)]
    pub can_invite_users: bool,
    #[serde(default)]
    pub can_restrict_members: bool,
    #[serde(default)]
    pub can_pin_messages: bool,
    #[serde(default)]
    pub can_manage_topics: bool,
    #[serde(default)]
    pub can_promote_members: bool,
    #[serde(default)]
    pub can_manage_video_chats: bool,
    #[serde(default)]
    pub can_post_stories: bool,
    #[serde(default)]
    pub can_edit_stories: bool,
    #[serde(default)]
    pub can_delete_stories: bool,
    #[serde(default)]
    pub is_anonymous: bool,
}

impl AdministratorRights {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "@type": "chatAdministratorRights",
            "can_manage_chat": self.can_manage_chat,
            "can_change_info": self.can_change_info,
            "can_post_messages": self.can_post_messages,
            "can_edit_messages": self.can_edit_messages,
            "can_delete_messages": self.can_delete_messages,
            "can_invite_users": self.can_invite_users,
            "can_restrict_members": self.can_restrict_members,
            "can_pin_messages": self.can_pin_messages,
            "can_manage_topics": self.can_manage_topics,
            "can_promote_members": self.can_promote_members,
            "can_manage_video_chats": self.can_manage_video_chats,
            "can_post_stories": self.can_post_stories,
            "can_edit_stories": self.can_edit_stories,
            "can_delete_stories": self.can_delete_stories,
            "is_anonymous": self.is_anonymous,
        })
    }
}

/// What a chat's members may do (chatPermissions): the group's defaults, or one member's. Every
/// field, so that the group's can be sent back changed (setChatPermissions, admin.rs).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ChatPermissions {
    pub can_send_basic_messages: bool,
    #[serde(default)]
    pub can_send_audios: bool,
    #[serde(default)]
    pub can_send_documents: bool,
    #[serde(default)]
    pub can_send_photos: bool,
    #[serde(default)]
    pub can_send_videos: bool,
    #[serde(default)]
    pub can_send_video_notes: bool,
    #[serde(default)]
    pub can_send_voice_notes: bool,
    #[serde(default)]
    pub can_send_polls: bool,
    /// Stickers, GIFs, games and inline bots' results.
    #[serde(default)]
    pub can_send_other_messages: bool,
    #[serde(default)]
    pub can_add_link_previews: bool,
    #[serde(default)]
    pub can_react_to_messages: bool,
    #[serde(default)]
    pub can_edit_tag: bool,
    #[serde(default)]
    pub can_change_info: bool,
    #[serde(default)]
    pub can_invite_users: bool,
    #[serde(default)]
    pub can_pin_messages: bool,
    #[serde(default)]
    pub can_create_topics: bool,
}

impl ChatPermissions {
    /// As TDLib takes them back.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "@type": "chatPermissions",
            "can_send_basic_messages": self.can_send_basic_messages, "can_send_audios": self.can_send_audios,
            "can_send_documents": self.can_send_documents, "can_send_photos": self.can_send_photos,
            "can_send_videos": self.can_send_videos, "can_send_video_notes": self.can_send_video_notes,
            "can_send_voice_notes": self.can_send_voice_notes, "can_send_polls": self.can_send_polls,
            "can_send_other_messages": self.can_send_other_messages, "can_add_link_previews": self.can_add_link_previews,
            "can_react_to_messages": self.can_react_to_messages, "can_edit_tag": self.can_edit_tag,
            "can_change_info": self.can_change_info, "can_invite_users": self.can_invite_users,
            "can_pin_messages": self.can_pin_messages, "can_create_topics": self.can_create_topics,
        })
    }
}

// ---- chats ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: ChatType,
    pub title: String,
    #[serde(default)]
    pub photo: Option<ChatPhoto>,
    pub permissions: ChatPermissions,
    #[serde(default)]
    pub last_message: Option<Box<Message>>,
    pub positions: Vec<ChatPosition>,
    pub is_marked_as_unread: bool,
    pub unread_count: i32,
    pub last_read_inbox_message_id: i64,
    pub last_read_outbox_message_id: i64,
    pub unread_mention_count: i32,
    pub notification_settings: ChatNotificationSettings,
    /// A user we have blocked, from messages or only from stories; none otherwise.
    #[serde(default)]
    pub block_list: Option<BlockList>,
    #[serde(default)]
    pub can_be_deleted_only_for_self: bool,
    #[serde(default)]
    pub can_be_deleted_for_all_users: bool,
    #[serde(default)]
    pub can_be_reported: bool,
    /// Its content may not be saved, forwarded or captured: the window hides from screen
    /// capture while the chat is open (conversation.rs).
    #[serde(default)]
    pub has_protected_content: bool,
    /// The message whose keyboard (a bot's) the chat shows under the composer; 0 for none.
    #[serde(default)]
    pub reply_markup_message_id: i64,
    /// Which reactions the chat allows (reactions.rs); none while TDLib has not said.
    #[serde(default)]
    pub available_reactions: Option<ChatAvailableReactions>,
    /// Words left in the composer, kept by Telegram for every device (drafts.rs).
    #[serde(default)]
    pub draft_message: Option<DraftMessage>,
    /// People waiting for an administrator's approval to join (manage.rs); none for no requests.
    #[serde(default)]
    pub pending_join_requests: Option<ChatJoinRequestsInfo>,
}

/// Requests to join a chat that wait for approval (chatJoinRequestsInfo).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ChatJoinRequestsInfo {
    pub total_count: i32,
}

/// Which reactions a chat allows: all of Telegram's, or some of them; and how many one person
/// may give a message here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatAvailableReactions {
    #[serde(rename = "chatAvailableReactionsAll")]
    All {
        #[serde(default)]
        max_reaction_count: i32,
    },
    #[serde(rename = "chatAvailableReactionsSome")]
    Some {
        reactions: Vec<ReactionType>,
        #[serde(default)]
        max_reaction_count: i32,
    },
}

/// A draft: words left in a chat's composer, with when.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DraftMessage {
    #[serde(default)]
    pub date: i32,
    pub content: DraftMessageContent,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum DraftMessageContent {
    #[serde(rename = "draftMessageContentText")]
    Text { text: FormattedText },
    /// A voice or video note waiting, or a rich message: not written here.
    #[serde(other)]
    Other,
}

impl DraftMessage {
    /// The words of a text draft; "" for any other.
    pub fn words(&self) -> &str {
        match &self.content {
            DraftMessageContent::Text { text } => &text.text,
            DraftMessageContent::Other => "",
        }
    }
}

/// Where a user is blocked (TDLib's BlockList): from sending us messages, or only from our stories.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum BlockList {
    #[serde(rename = "blockListMain")]
    Main,
    #[serde(rename = "blockListStories")]
    Stories,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatType {
    #[serde(rename = "chatTypePrivate")]
    Private { user_id: i64 },
    #[serde(rename = "chatTypeBasicGroup")]
    BasicGroup { basic_group_id: i64 },
    #[serde(rename = "chatTypeSupergroup")]
    Supergroup { supergroup_id: i64, is_channel: bool },
    #[serde(rename = "chatTypeSecret")]
    Secret { user_id: i64 },
}

/// The photo of a chat or of a person (TDLib's chatPhotoInfo and profilePhoto): a square in two
/// sizes, 160 and 640 pixels, with a tiny preview of a few pixels. Downloaded like any file
/// (avatars.rs).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChatPhoto {
    pub small: File,
    pub big: File,
    #[serde(default)]
    pub minithumbnail: Option<Minithumbnail>,
}

/// A chat list: the main one, the archive, or one of the account's folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatList {
    #[serde(rename = "chatListMain")]
    Main,
    #[serde(rename = "chatListArchive")]
    Archive,
    #[serde(rename = "chatListFolder")]
    Folder { chat_folder_id: i32 },
}

impl ChatList {
    /// The list as TDLib takes it in a request.
    pub fn to_json(self) -> serde_json::Value {
        match self {
            ChatList::Main => serde_json::json!({ "@type": "chatListMain" }),
            ChatList::Archive => serde_json::json!({ "@type": "chatListArchive" }),
            ChatList::Folder { chat_folder_id } => {
                serde_json::json!({ "@type": "chatListFolder", "chat_folder_id": chat_folder_id })
            }
        }
    }
}

/// Where a chat is in a list: chats are sorted by `order`, the highest first; 0 means the chat is
/// not in the list.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ChatPosition {
    pub list: ChatList,
    #[serde(with = "int64")]
    pub order: i64,
    pub is_pinned: bool,
}

/// A chat's notification settings. All of them are read, so that muting sends them back unchanged
/// but for the mute.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct ChatNotificationSettings {
    pub use_default_mute_for: bool,
    pub mute_for: i32,
    pub use_default_sound: bool,
    #[serde(with = "int64")]
    pub sound_id: i64,
    pub use_default_show_preview: bool,
    pub show_preview: bool,
    pub use_default_mute_stories: bool,
    pub mute_stories: bool,
    pub use_default_story_sound: bool,
    #[serde(with = "int64")]
    pub story_sound_id: i64,
    pub use_default_show_story_poster: bool,
    pub show_story_poster: bool,
    pub use_default_disable_pinned_message_notifications: bool,
    pub disable_pinned_message_notifications: bool,
    pub use_default_disable_mention_notifications: bool,
    pub disable_mention_notifications: bool,
}

impl ChatNotificationSettings {
    /// These settings with `mute_for`, as TDLib takes them (setChatNotificationSettings).
    pub fn with_mute_for(&self, mute_for: i32) -> serde_json::Value {
        serde_json::json!({
            "@type": "chatNotificationSettings",
            "use_default_mute_for": false,
            "mute_for": mute_for,
            "use_default_sound": self.use_default_sound,
            "sound_id": self.sound_id.to_string(),
            "use_default_show_preview": self.use_default_show_preview,
            "show_preview": self.show_preview,
            "use_default_mute_stories": self.use_default_mute_stories,
            "mute_stories": self.mute_stories,
            "use_default_story_sound": self.use_default_story_sound,
            "story_sound_id": self.story_sound_id.to_string(),
            "use_default_show_story_poster": self.use_default_show_story_poster,
            "show_story_poster": self.show_story_poster,
            "use_default_disable_pinned_message_notifications": self.use_default_disable_pinned_message_notifications,
            "disable_pinned_message_notifications": self.disable_pinned_message_notifications,
            "use_default_disable_mention_notifications": self.use_default_disable_mention_notifications,
            "disable_mention_notifications": self.disable_mention_notifications,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(tag = "@type")]
/// Which chats a default notification setting is for.
pub enum NotificationSettingsScope {
    #[serde(rename = "notificationSettingsScopePrivateChats")]
    Private,
    #[serde(rename = "notificationSettingsScopeGroupChats")]
    Group,
    #[serde(rename = "notificationSettingsScopeChannelChats")]
    Channel,
}

/// The notification settings of a kind of chat, which its chats follow unless they say otherwise.
/// All of them are read, so that switching a kind off or on sends them back unchanged but for the
/// mute.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct ScopeNotificationSettings {
    pub mute_for: i32,
    #[serde(with = "int64")]
    pub sound_id: i64,
    pub show_preview: bool,
    pub use_default_mute_stories: bool,
    pub mute_stories: bool,
    #[serde(with = "int64")]
    pub story_sound_id: i64,
    pub show_story_poster: bool,
    pub disable_pinned_message_notifications: bool,
    pub disable_mention_notifications: bool,
}

impl ScopeNotificationSettings {
    /// These settings with `mute_for`, as TDLib takes them (setScopeNotificationSettings).
    pub fn with_mute_for(&self, mute_for: i32) -> serde_json::Value {
        serde_json::json!({
            "@type": "scopeNotificationSettings",
            "mute_for": mute_for,
            "sound_id": self.sound_id.to_string(),
            "show_preview": self.show_preview,
            "use_default_mute_stories": self.use_default_mute_stories,
            "mute_stories": self.mute_stories,
            "story_sound_id": self.story_sound_id.to_string(),
            "show_story_poster": self.show_story_poster,
            "disable_pinned_message_notifications": self.disable_pinned_message_notifications,
            "disable_mention_notifications": self.disable_mention_notifications,
        })
    }
}

impl NotificationSettingsScope {
    /// The scope as TDLib takes it in a request.
    pub fn to_json(self) -> serde_json::Value {
        match self {
            NotificationSettingsScope::Private => serde_json::json!({ "@type": "notificationSettingsScopePrivateChats" }),
            NotificationSettingsScope::Group => serde_json::json!({ "@type": "notificationSettingsScopeGroupChats" }),
            NotificationSettingsScope::Channel => serde_json::json!({ "@type": "notificationSettingsScopeChannelChats" }),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatFolderInfo {
    pub id: i32,
    pub name: ChatFolderName,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatFolderName {
    pub text: FormattedText,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatAction {
    #[serde(rename = "chatActionTyping")]
    Typing,
    #[serde(rename = "chatActionCancel")]
    Cancel,
    #[serde(other)]
    Other,
}

// ---- messages ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: i64,
    pub sender_id: MessageSender,
    pub chat_id: i64,
    #[serde(default)]
    pub sending_state: Option<MessageSendingState>,
    pub is_outgoing: bool,
    pub date: i32,
    pub edit_date: i32,
    /// The messages of an album (photos and videos sent together) share it; 0 for none.
    #[serde(default, with = "int64")]
    pub media_album_id: i64,
    /// Who first wrote it, when it was forwarded.
    #[serde(default)]
    pub forward_info: Option<MessageForwardInfo>,
    /// The message it replies to.
    #[serde(default)]
    pub reply_to: Option<MessageReplyTo>,
    /// Its content self-destructs: the moment it is opened, or so long after.
    #[serde(default)]
    pub self_destruct_type: Option<MessageSelfDestructType>,
    #[serde(default)]
    pub is_pinned: bool,
    /// A bot's buttons under it, or the keyboard it asks the chat to show.
    #[serde(default)]
    pub reply_markup: Option<ReplyMarkup>,
    /// Its views, forwards and reactions.
    #[serde(default)]
    pub interaction_info: Option<MessageInteractionInfo>,
    /// The topic it is in, in a group with topics (topics.rs).
    #[serde(default)]
    pub topic_id: Option<MessageTopic>,
    pub content: MessageContent,
}

impl Message {
    /// The reactions to it, as TDLib lists them.
    pub fn reactions(&self) -> &[MessageReaction] {
        self.interaction_info.as_ref().and_then(|info| info.reactions.as_ref()).map(|reactions| reactions.reactions.as_slice()).unwrap_or(&[])
    }

    /// The forum topic it is in, when it is in one.
    pub fn forum_topic_id(&self) -> Option<i32> {
        match self.topic_id {
            Some(MessageTopic::Forum { forum_topic_id }) => Some(forum_topic_id),
            _ => None,
        }
    }
}

/// What others did with a message: its views, its forwards, the reactions to it.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct MessageInteractionInfo {
    #[serde(default)]
    pub view_count: i32,
    #[serde(default)]
    pub forward_count: i32,
    #[serde(default)]
    pub reactions: Option<MessageReactions>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct MessageReactions {
    pub reactions: Vec<MessageReaction>,
    /// Saved Messages: the reactions are tags, not reactions.
    #[serde(default)]
    pub are_tags: bool,
}

/// One reaction to a message: which, how many gave it, and whether we did.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MessageReaction {
    #[serde(rename = "type")]
    pub kind: ReactionType,
    pub total_count: i32,
    pub is_chosen: bool,
}

/// A reaction: an emoji, a custom emoji (a sticker we know by its id), or a paid one (stars).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(tag = "@type")]
pub enum ReactionType {
    #[serde(rename = "reactionTypeEmoji")]
    Emoji { emoji: String },
    #[serde(rename = "reactionTypeCustomEmoji")]
    CustomEmoji {
        #[serde(with = "int64")]
        custom_emoji_id: i64,
    },
    #[serde(rename = "reactionTypePaid")]
    Paid,
}

impl ReactionType {
    /// As TDLib takes it back (addMessageReaction, removeMessageReaction).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ReactionType::Emoji { emoji } => serde_json::json!({ "@type": "reactionTypeEmoji", "emoji": emoji }),
            ReactionType::CustomEmoji { custom_emoji_id } => {
                serde_json::json!({ "@type": "reactionTypeCustomEmoji", "custom_emoji_id": custom_emoji_id.to_string() })
            }
            ReactionType::Paid => serde_json::json!({ "@type": "reactionTypePaid" }),
        }
    }
}

/// Which reactions may be given to a message, as Telegram orders them for the picker.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AvailableReactions {
    #[serde(default)]
    pub top_reactions: Vec<AvailableReaction>,
    #[serde(default)]
    pub recent_reactions: Vec<AvailableReaction>,
    #[serde(default)]
    pub popular_reactions: Vec<AvailableReaction>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AvailableReaction {
    #[serde(rename = "type")]
    pub kind: ReactionType,
    #[serde(default)]
    pub needs_premium: bool,
}

/// Where a message is within its chat: a forum topic, or something else (a thread of direct
/// messages, Saved Messages' tags) that FinchGram does not tell apart.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageTopic {
    #[serde(rename = "messageTopicForum")]
    Forum { forum_topic_id: i32 },
    #[serde(other)]
    Other,
}

// ---- forum topics -----------------------------------------------------------------------------

/// getForumTopics' answer: a page of a group's topics, newest activity first.
#[derive(Debug, Clone, Deserialize)]
pub struct ForumTopics {
    pub topics: Vec<ForumTopic>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForumTopic {
    pub info: ForumTopicInfo,
    #[serde(default)]
    pub last_message: Option<Box<Message>>,
    #[serde(default, with = "int64")]
    pub order: i64,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub unread_count: i32,
    #[serde(default)]
    pub unread_mention_count: i32,
    #[serde(default)]
    pub last_read_inbox_message_id: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ForumTopicInfo {
    pub chat_id: i64,
    pub forum_topic_id: i32,
    pub name: String,
    pub icon: ForumTopicIcon,
    #[serde(default)]
    pub is_general: bool,
    #[serde(default)]
    pub is_outgoing: bool,
    #[serde(default)]
    pub is_closed: bool,
    #[serde(default)]
    pub is_hidden: bool,
}

/// A topic's icon: one of Telegram's colours (0x6FB9F0, 0xFFD67E, 0xCB86DB, 0x8EEE98, 0xFF93B2,
/// 0xFB6F5F), and a custom emoji drawn on it, 0 for none.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ForumTopicIcon {
    #[serde(default)]
    pub color: i32,
    #[serde(default, with = "int64")]
    pub custom_emoji_id: i64,
}

/// A list of stickers (getForumTopicDefaultIcons, getCustomEmojiStickers).
#[derive(Debug, Clone, Deserialize)]
pub struct Stickers {
    pub stickers: Vec<Sticker>,
}

/// What a bot attached to a message: buttons under it (an inline keyboard), a keyboard for the
/// chat to show under the composer, that keyboard taken away, or a reply asked for.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ReplyMarkup {
    #[serde(rename = "replyMarkupInlineKeyboard")]
    InlineKeyboard { rows: Vec<Vec<InlineKeyboardButton>> },
    #[serde(rename = "replyMarkupShowKeyboard")]
    ShowKeyboard {
        rows: Vec<Vec<KeyboardButton>>,
        #[serde(default)]
        is_persistent: bool,
        #[serde(default)]
        resize_keyboard: bool,
        #[serde(default)]
        one_time: bool,
        #[serde(default)]
        is_personal: bool,
        #[serde(default)]
        input_field_placeholder: String,
    },
    #[serde(rename = "replyMarkupRemoveKeyboard")]
    RemoveKeyboard {
        #[serde(default)]
        is_personal: bool,
    },
    #[serde(rename = "replyMarkupForceReply")]
    ForceReply {
        #[serde(default)]
        is_personal: bool,
        #[serde(default)]
        input_field_placeholder: String,
    },
}

/// A button under a bot's message.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    #[serde(rename = "type")]
    pub kind: InlineKeyboardButtonType,
}

/// What a button under a message does when pressed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum InlineKeyboardButtonType {
    #[serde(rename = "inlineKeyboardButtonTypeUrl")]
    Url { url: String },
    /// Logging in to a site through Telegram: asked about first (getLoginUrlInfo).
    #[serde(rename = "inlineKeyboardButtonTypeLoginUrl")]
    LoginUrl { url: String, id: i64, forward_text: String },
    #[serde(rename = "inlineKeyboardButtonTypeWebApp")]
    WebApp { url: String },
    /// `data` is TDLib's bytes, base64 in its JSON.
    #[serde(rename = "inlineKeyboardButtonTypeCallback")]
    Callback { data: String },
    #[serde(rename = "inlineKeyboardButtonTypeCallbackWithPassword")]
    CallbackWithPassword { data: String },
    #[serde(rename = "inlineKeyboardButtonTypeCallbackGame")]
    CallbackGame,
    #[serde(rename = "inlineKeyboardButtonTypeSwitchInline")]
    SwitchInline { query: String, target_chat: TargetChat },
    #[serde(rename = "inlineKeyboardButtonTypeBuy")]
    Buy,
    #[serde(rename = "inlineKeyboardButtonTypeUser")]
    User { user_id: i64 },
    #[serde(rename = "inlineKeyboardButtonTypeCopyText")]
    CopyText { text: String },
    #[serde(other)]
    Disabled,
}

/// Where a switch-to-inline button takes the words: this chat, or one the user chooses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum TargetChat {
    #[serde(rename = "targetChatCurrent")]
    Current,
    #[serde(other)]
    Chosen,
}

/// A button of a bot's keyboard under the composer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct KeyboardButton {
    pub text: String,
    #[serde(rename = "type")]
    pub kind: KeyboardButtonType,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum KeyboardButtonType {
    /// The words are sent as a message.
    #[serde(rename = "keyboardButtonTypeText")]
    Text,
    #[serde(rename = "keyboardButtonTypeRequestPhoneNumber")]
    RequestPhoneNumber,
    #[serde(rename = "keyboardButtonTypeRequestLocation")]
    RequestLocation,
    #[serde(rename = "keyboardButtonTypeRequestPoll")]
    RequestPoll {},
    #[serde(rename = "keyboardButtonTypeRequestUsers")]
    RequestUsers {},
    #[serde(rename = "keyboardButtonTypeRequestChat")]
    RequestChat {},
    #[serde(rename = "keyboardButtonTypeWebApp")]
    WebApp { url: String },
    #[serde(other)]
    Other,
}

/// A bot's answer to a button pressed (getCallbackQueryAnswer): words to show, as a notice or as
/// a dialog, or a link to open; or nothing.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CallbackQueryAnswer {
    pub text: String,
    pub show_alert: bool,
    pub url: String,
}

/// What a login button leads to (getLoginUrlInfo): the site at once, or a question first.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum LoginUrlInfo {
    #[serde(rename = "loginUrlInfoOpen")]
    Open { url: String },
    #[serde(rename = "loginUrlInfoRequestConfirmation")]
    RequestConfirmation { url: String, domain: String, bot_user_id: i64, request_write_access: bool },
}

/// A link (getLoginUrl).
#[derive(Debug, Clone, Deserialize)]
pub struct HttpUrl {
    pub url: String,
}

/// A bot's web app, opened (openWebApp).
#[derive(Debug, Clone, Deserialize)]
pub struct WebAppInfo {
    pub url: WebAppUrl,
}

/// A bot's web app's address (getWebAppUrl).
#[derive(Debug, Clone, Deserialize)]
pub struct WebAppUrl {
    pub url: String,
}

/// What a bot says it can do: the button at the composer's left, and its commands.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BotInfo {
    /// What the bot does, as its maker wrote it (the info panel's description row).
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub menu_button: Option<BotMenuButton>,
    #[serde(default)]
    pub commands: Vec<BotCommand>,
}

/// The button at the composer's left: a web app with a name of its own, or (empty) the commands.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BotMenuButton {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BotCommand {
    pub command: String,
    #[serde(default)]
    pub description: String,
}

/// The commands of one bot in a group.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BotCommands {
    pub bot_user_id: i64,
    #[serde(default)]
    pub commands: Vec<BotCommand>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BasicGroupFullInfo {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub members: Vec<ChatMember>,
    /// The group's primary invite link, for an administrator who may invite; none otherwise.
    #[serde(default)]
    pub invite_link: Option<ChatInviteLink>,
    #[serde(default)]
    pub bot_commands: Vec<BotCommands>,
}

/// How a message's content self-destructs once the user opens it: at once (it can be seen only
/// once, and stays only while it is open), or after `self_destruct_time` seconds.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageSelfDestructType {
    #[serde(rename = "messageSelfDestructTypeTimer")]
    Timer { self_destruct_time: i32 },
    #[serde(rename = "messageSelfDestructTypeImmediately")]
    Immediately,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MessageForwardInfo {
    pub origin: MessageOrigin,
}

/// Who first wrote a message that was forwarded, or that a reply from another chat quotes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageOrigin {
    #[serde(rename = "messageOriginUser")]
    User { sender_user_id: i64 },
    /// Someone whose privacy settings hide who they are: only their name.
    #[serde(rename = "messageOriginHiddenUser")]
    HiddenUser { sender_name: String },
    #[serde(rename = "messageOriginChat")]
    Chat { sender_chat_id: i64 },
    #[serde(rename = "messageOriginChannel")]
    Channel { chat_id: i64 },
}

/// What a message replies to: a message (in this chat, or quoted from another one), or a story.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageReplyTo {
    #[serde(rename = "messageReplyToMessage")]
    Message {
        /// 0 when the message is in a chat we do not know.
        chat_id: i64,
        message_id: i64,
        /// The part of it the reply quotes.
        #[serde(default)]
        quote: Option<TextQuote>,
        /// Who wrote it, when it is in another chat.
        #[serde(default)]
        origin: Option<MessageOrigin>,
        /// Its photo, video, … when it is in another chat.
        #[serde(default)]
        content: Option<Box<MessageContent>>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TextQuote {
    pub text: FormattedText,
}

/// What can be done with a message (getMessageProperties), as TDLib decides it: what Telegram's own
/// apps offer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct MessageProperties {
    pub can_be_deleted_only_for_self: bool,
    pub can_be_deleted_for_all_users: bool,
    pub can_be_edited: bool,
    pub can_be_forwarded: bool,
    pub can_be_replied: bool,
    /// False where the chat restricts saving content: then it cannot be copied either.
    pub can_be_saved: bool,
    pub can_get_link: bool,
    pub can_report_chat: bool,
}

/// A message's link (getMessageLink).
#[derive(Debug, Clone, Deserialize)]
pub struct MessageLink {
    pub link: String,
}

/// Where a report is (reportChat). Telegram's servers lead it: a choice of reasons, maybe another
/// one under it, maybe words to add.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ReportChatResult {
    #[serde(rename = "reportChatResultOk")]
    Ok,
    #[serde(rename = "reportChatResultOptionRequired")]
    OptionRequired { title: String, options: Vec<ReportOption> },
    /// `option_id` goes with the words; they may be left out when `is_optional`.
    #[serde(rename = "reportChatResultTextRequired")]
    TextRequired { option_id: String, is_optional: bool },
    #[serde(rename = "reportChatResultMessagesRequired")]
    MessagesRequired,
}

/// One reason to choose; its id is TDLib's bytes, which its JSON writes in base64.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReportOption {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageSender {
    #[serde(rename = "messageSenderUser")]
    User { user_id: i64 },
    #[serde(rename = "messageSenderChat")]
    Chat { chat_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageSendingState {
    #[serde(rename = "messageSendingStatePending")]
    Pending,
    #[serde(rename = "messageSendingStateFailed")]
    Failed { can_retry: bool },
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct FormattedText {
    pub text: String,
    /// Which parts of the text are bold, links and so on. Offsets and lengths count UTF-16 code
    /// units.
    #[serde(default)]
    pub entities: Vec<TextEntity>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TextEntity {
    pub offset: i32,
    pub length: i32,
    #[serde(rename = "type")]
    pub kind: TextEntityType,
}

/// What an entity makes of its part of the text. What FinchGram shows as plain text is `Other`:
/// hashtags, bot commands, phone numbers, spoilers (for now), custom emoji (their emoji is in the
/// text), and so on.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum TextEntityType {
    #[serde(rename = "textEntityTypeBold")]
    Bold,
    #[serde(rename = "textEntityTypeItalic")]
    Italic,
    #[serde(rename = "textEntityTypeUnderline")]
    Underline,
    #[serde(rename = "textEntityTypeStrikethrough")]
    Strikethrough,
    #[serde(rename = "textEntityTypeCode")]
    Code,
    #[serde(rename = "textEntityTypePre")]
    Pre,
    #[serde(rename = "textEntityTypePreCode")]
    PreCode {},
    #[serde(rename = "textEntityTypeBlockQuote")]
    BlockQuote,
    #[serde(rename = "textEntityTypeExpandableBlockQuote")]
    ExpandableBlockQuote,
    /// A web address as written in the text.
    #[serde(rename = "textEntityTypeUrl")]
    Url,
    /// Words that link to `url`.
    #[serde(rename = "textEntityTypeTextUrl")]
    TextUrl { url: String },
    #[serde(rename = "textEntityTypeEmailAddress")]
    EmailAddress,
    /// @username.
    #[serde(rename = "textEntityTypeMention")]
    Mention,
    /// A user's name, for a user without a username.
    #[serde(rename = "textEntityTypeMentionName")]
    MentionName { user_id: i64 },
    #[serde(other)]
    Other,
}

/// A link's preview, made by Telegram's servers (FinchGram never fetches the page itself).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    #[serde(default)]
    pub display_url: String,
    #[serde(default)]
    pub site_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: FormattedText,
    #[serde(default)]
    pub author: String,
    /// Not 0: Telegram has the page in its reading mode (Instant View).
    #[serde(default)]
    pub instant_view_version: i32,
}

/// What a message holds. Kinds FinchGram does not show yet are `Other`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageContent {
    #[serde(rename = "messageText")]
    Text {
        text: FormattedText,
        #[serde(default)]
        link_preview: Option<LinkPreview>,
    },
    /// `is_secret`: sent to self-destruct (seen once, or for a time): TDLib asks that it be shown
    /// blurred, and only once the user opens it (openMessageContent), which starts its end.
    #[serde(rename = "messagePhoto")]
    Photo {
        photo: Photo,
        caption: FormattedText,
        #[serde(default)]
        is_secret: bool,
    },
    #[serde(rename = "messageVideo")]
    Video {
        video: Video,
        caption: FormattedText,
        #[serde(default)]
        is_secret: bool,
    },
    #[serde(rename = "messageAnimation")]
    Animation {
        animation: Animation,
        caption: FormattedText,
        #[serde(default)]
        is_secret: bool,
    },
    #[serde(rename = "messageAudio")]
    Audio { audio: Audio, caption: FormattedText },
    #[serde(rename = "messageDocument")]
    Document { document: Document, caption: FormattedText },
    #[serde(rename = "messageVoiceNote")]
    VoiceNote {
        voice_note: VoiceNote,
        caption: FormattedText,
        /// Listened to by the user (an incoming one shows a dot until then).
        #[serde(default)]
        is_listened: bool,
    },
    #[serde(rename = "messageVideoNote")]
    VideoNote {
        video_note: VideoNote,
        #[serde(default)]
        is_viewed: bool,
        #[serde(default)]
        is_secret: bool,
    },
    /// Self-destructing content that is gone: it was opened once, or its time ran out.
    #[serde(rename = "messageExpiredPhoto")]
    ExpiredPhoto {},
    #[serde(rename = "messageExpiredVideo")]
    ExpiredVideo {},
    #[serde(rename = "messageExpiredVideoNote")]
    ExpiredVideoNote {},
    #[serde(rename = "messageExpiredVoiceNote")]
    ExpiredVoiceNote {},
    #[serde(rename = "messageSticker")]
    Sticker { sticker: Sticker },
    #[serde(rename = "messageAnimatedEmoji")]
    AnimatedEmoji { emoji: String },
    #[serde(rename = "messageDice")]
    Dice { emoji: String },
    #[serde(rename = "messageLocation")]
    Location {},
    #[serde(rename = "messageVenue")]
    Venue { venue: Venue },
    #[serde(rename = "messageContact")]
    Contact { contact: Contact },
    #[serde(rename = "messagePoll")]
    Poll { poll: Poll },
    #[serde(rename = "messageGame")]
    Game { game: Game },
    #[serde(rename = "messageInvoice")]
    Invoice {},
    #[serde(rename = "messageCall")]
    Call {},
    #[serde(rename = "messageStory")]
    Story {},
    #[serde(rename = "messageGift")]
    Gift {},
    #[serde(rename = "messageBasicGroupChatCreate")]
    BasicGroupChatCreate { title: String },
    #[serde(rename = "messageSupergroupChatCreate")]
    SupergroupChatCreate { title: String },
    #[serde(rename = "messageChatChangeTitle")]
    ChatChangeTitle { title: String },
    #[serde(rename = "messageChatChangePhoto")]
    ChatChangePhoto {},
    #[serde(rename = "messageChatDeletePhoto")]
    ChatDeletePhoto {},
    #[serde(rename = "messageChatAddMembers")]
    ChatAddMembers { member_user_ids: Vec<i64> },
    #[serde(rename = "messageChatJoinByLink")]
    ChatJoinByLink {},
    #[serde(rename = "messageChatJoinByRequest")]
    ChatJoinByRequest {},
    #[serde(rename = "messageChatDeleteMember")]
    ChatDeleteMember { user_id: i64 },
    #[serde(rename = "messagePinMessage")]
    PinMessage {},
    #[serde(rename = "messageScreenshotTaken")]
    ScreenshotTaken {},
    #[serde(rename = "messageContactRegistered")]
    ContactRegistered {},
    #[serde(other)]
    Other,
}

/// A file TDLib knows of: on Telegram's servers, and on disk once downloaded (files.rs); one of
/// ours on its way up (attachments.rs).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct File {
    pub id: i32,
    /// 0 until known; `expected_size` is TDLib's guess meanwhile.
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub expected_size: i64,
    pub local: LocalFile,
    #[serde(default)]
    pub remote: RemoteFile,
}

impl File {
    /// Its size, or TDLib's guess of it.
    pub fn size_or_expected(&self) -> i64 {
        if self.size > 0 { self.size } else { self.expected_size }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LocalFile {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub is_downloading_active: bool,
    #[serde(default)]
    pub is_downloading_completed: bool,
    #[serde(default)]
    pub downloaded_size: i64,
}

/// The file on Telegram's servers: how much of ours has reached them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RemoteFile {
    #[serde(default)]
    pub is_uploading_active: bool,
    #[serde(default)]
    pub is_uploading_completed: bool,
    #[serde(default)]
    pub uploaded_size: i64,
}

/// A photo in several sizes, smallest first, with a tiny preview in the message itself.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Photo {
    #[serde(default)]
    pub minithumbnail: Option<Minithumbnail>,
    pub sizes: Vec<PhotoSize>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PhotoSize {
    pub photo: File,
    pub width: i32,
    pub height: i32,
}

/// A JPEG about 40 pixels wide, in the message itself: base64 in TDLib's JSON.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Minithumbnail {
    pub width: i32,
    pub height: i32,
    pub data: String,
}

/// A video's or a GIF's still picture, downloaded like any file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Thumbnail {
    pub format: ThumbnailFormat,
    pub width: i32,
    pub height: i32,
    pub file: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ThumbnailFormat {
    #[serde(rename = "thumbnailFormatJpeg")]
    Jpeg,
    #[serde(rename = "thumbnailFormatPng")]
    Png,
    #[serde(rename = "thumbnailFormatWebp")]
    Webp,
    /// GIF, MPEG-4, WebM, TGS: moving pictures, for a player.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Video {
    pub duration: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub minithumbnail: Option<Minithumbnail>,
    #[serde(default)]
    pub thumbnail: Option<Thumbnail>,
    pub video: File,
}

/// A GIF, as Telegram keeps it (usually a silent MPEG-4 video).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Animation {
    pub duration: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub minithumbnail: Option<Minithumbnail>,
    #[serde(default)]
    pub thumbnail: Option<Thumbnail>,
    pub animation: File,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Audio {
    pub title: String,
    pub performer: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Document {
    pub file_name: String,
    #[serde(default)]
    pub mime_type: String,
    pub document: File,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Sticker {
    /// Its id; a custom emoji is known by it (a topic's icon, a reaction).
    #[serde(default, with = "int64")]
    pub id: i64,
    pub width: i32,
    pub height: i32,
    pub emoji: String,
    pub format: StickerFormat,
    /// A still picture of it, in WebP or JPEG; may be missing.
    pub thumbnail: Option<Thumbnail>,
    pub sticker: File,
}

/// What a sticker's file is: a still WebP picture, a Lottie animation (TGS) or a WebM video.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum StickerFormat {
    #[serde(rename = "stickerFormatWebp")]
    Webp,
    #[serde(rename = "stickerFormatTgs")]
    Tgs,
    #[serde(rename = "stickerFormatWebm")]
    Webm,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Venue {
    pub title: String,
}

/// A round video message: a square video shown in a circle (the design: 180 px).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VideoNote {
    pub duration: i32,
    /// Its width and height, which are the same.
    pub length: i32,
    #[serde(default)]
    pub minithumbnail: Option<Minithumbnail>,
    #[serde(default)]
    pub thumbnail: Option<Thumbnail>,
    pub video: File,
}

/// A voice message's sound (voiceNote): Opus in OGG from Telegram's own apps, MP3 or M4A from
/// others, FinchGram's among them (src/telegram/voice.rs).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VoiceNote {
    /// In seconds, as the sender says.
    pub duration: i32,
    /// Its loudness as values of 5 bits, packed, in base64 (TDLib's bytes); "" when the sender
    /// gave none.
    #[serde(default)]
    pub waveform: String,
    #[serde(default)]
    pub mime_type: String,
    /// Telegram's transcription of it, once asked for (recognizeSpeech, src/telegram/voice.rs): on
    /// its way, done, or failed; none until then.
    #[serde(default)]
    pub speech_recognition_result: Option<SpeechRecognitionResult>,
    pub voice: File,
}

/// Telegram's own transcription of a voice message: Premium's, or one of the few a week without
/// it, or a group's boosted for it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum SpeechRecognitionResult {
    #[serde(rename = "speechRecognitionResultPending")]
    Pending {
        #[serde(default)]
        partial_text: String,
    },
    #[serde(rename = "speechRecognitionResultText")]
    Text { text: String },
    #[serde(rename = "speechRecognitionResultError")]
    Error { error: SpeechError },
    #[serde(other)]
    Other,
}

/// Why Telegram could not transcribe it ("MSG_VOICE_TOO_LONG", …).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SpeechError {
    pub code: i32,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Contact {
    #[serde(default)]
    pub phone_number: String,
    pub first_name: String,
    pub last_name: String,
    #[serde(default)]
    pub vcard: String,
    /// The Telegram user with the number, 0 when they have no account (or hide it).
    #[serde(default)]
    pub user_id: i64,
}

impl Contact {
    pub fn name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name).trim().to_string()
    }

    /// As TDLib takes it (inputMessageContact, addContact).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "@type": "contact", "phone_number": self.phone_number, "first_name": self.first_name, "last_name": self.last_name,
            "vcard": self.vcard, "user_id": self.user_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Poll {
    pub question: FormattedText,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Game {
    pub title: String,
    /// What a game button asks the bot about (callbackQueryPayloadGame).
    #[serde(default)]
    pub short_name: String,
}

/// getChatHistory's answer. TDLib may leave out messages it no longer has.
#[derive(Debug, Clone, Deserialize)]
pub struct Messages {
    pub messages: Vec<Option<Message>>,
}

/// What searchChatMessages finds: the messages, newest first; how many there are in all (-1 when
/// TDLib does not know); and the message the next page starts from, 0 when these are all.
#[derive(Debug, Clone, Deserialize)]
pub struct FoundChatMessages {
    #[serde(default)]
    pub total_count: i32,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub next_from_message_id: i64,
}

/// A sponsored message in a channel: Telegram's own, shown as it comes.
#[derive(Debug, Clone, Deserialize)]
pub struct SponsoredMessage {
    pub message_id: i64,
    pub content: MessageContent,
    pub sponsor: AdvertisementSponsor,
    pub title: String,
    pub button_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdvertisementSponsor {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SponsoredMessages {
    pub messages: Vec<SponsoredMessage>,
}

// ---- finding people and chats, making chats ---------------------------------------------------

/// A list of chats by their ids (searchPublicChats, getChats). TDLib sends each chat itself
/// (updateNewChat) before it answers.
#[derive(Debug, Clone, Deserialize)]
pub struct Chats {
    pub chat_ids: Vec<i64>,
}

/// A list of users by their ids (getContacts, searchContacts); each user was sent before.
#[derive(Debug, Clone, Deserialize)]
pub struct Users {
    pub user_ids: Vec<i64>,
}

/// importContacts's answer: each contact's user, in the order given; 0 for a number that has no
/// account.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportedContacts {
    pub user_ids: Vec<i64>,
}

/// What searchMessages finds across the chats: the messages, newest first (its next_offset, for
/// the page after, is not asked for: one page is shown).
#[derive(Debug, Clone, Deserialize)]
pub struct FoundMessages {
    pub messages: Vec<Message>,
}

/// createNewBasicGroupChat's answer: the chat made.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatedBasicGroupChat {
    pub chat_id: i64,
}

/// Whether a username can be a chat's (checkChatUsername).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum CheckChatUsernameResult {
    #[serde(rename = "checkChatUsernameResultOk")]
    Ok,
    #[serde(rename = "checkChatUsernameResultUsernameInvalid")]
    UsernameInvalid,
    #[serde(rename = "checkChatUsernameResultUsernameOccupied")]
    UsernameOccupied,
    /// Taken, and for sale on fragment.com.
    #[serde(rename = "checkChatUsernameResultUsernamePurchasable")]
    UsernamePurchasable,
    #[serde(rename = "checkChatUsernameResultPublicChatsTooMany")]
    PublicChatsTooMany,
    #[serde(rename = "checkChatUsernameResultPublicGroupsUnavailable")]
    PublicGroupsUnavailable,
}

/// joinChat's answer: in, or waiting for an administrator (a group that approves its members), or
/// turned away.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatJoinResult {
    #[serde(rename = "chatJoinResultSuccess")]
    Success { chat_id: i64 },
    #[serde(rename = "chatJoinResultRequestSent")]
    RequestSent,
    #[serde(other)]
    Other,
}

// ---- notifications ----------------------------------------------------------------------------

/// A notification TDLib made, with an id that stays the same across starts.
#[derive(Debug, Clone, Deserialize)]
pub struct Notification {
    pub id: i32,
    /// Sent without sound by its sender.
    pub is_silent: bool,
    #[serde(rename = "type")]
    pub kind: NotificationType,
}

/// What a notification is about. FinchGram shows those about messages; new secret chats and
/// calls are `Other`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "@type")]
pub enum NotificationType {
    /// `show_preview` is false when the chat's settings say its messages are not shown in
    /// notifications.
    #[serde(rename = "notificationTypeNewMessage")]
    NewMessage { message: Box<Message>, show_preview: bool },
    #[serde(other)]
    Other,
}

/// A chat's notifications that TDLib still shows.
#[derive(Debug, Clone, Deserialize)]
pub struct NotificationGroup {
    pub id: i32,
    pub notifications: Vec<Notification>,
}

// ---- logging in -------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Countries {
    pub countries: Vec<CountryInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CountryInfo {
    pub country_code: String,
    pub name: String,
    pub english_name: String,
    pub is_hidden: bool,
    pub calling_codes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhoneNumberInfo {
    pub country_calling_code: String,
    pub formatted_phone_number: String,
}

/// A plain text answer (getCountryCode).
#[derive(Debug, Clone, Deserialize)]
pub struct Text {
    pub text: String,
}

/// TDLib's JSON writes int64 values as strings: they do not fit in a JavaScript number.
mod int64 {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_for_a_password_says_whether_it_can_be_recovered() {
        let state: AuthorizationState = serde_json::from_str(
            r#"{"@type":"authorizationStateWaitPassword","password_hint":"bird","has_recovery_email_address":true,"has_passport_data":false,"recovery_email_address_pattern":"z**@gmail.com"}"#,
        )
        .unwrap();
        assert_eq!(
            state,
            AuthorizationState::WaitPassword {
                password_hint: "bird".into(),
                has_recovery_email_address: true,
                recovery_email_address_pattern: "z**@gmail.com".into(),
            }
        );
        let state: AuthorizationState = serde_json::from_str(
            r#"{"@type":"authorizationStateWaitRegistration","terms_of_service":{"@type":"termsOfService","text":{"@type":"formattedText","text":"…","entities":[]},"min_user_age":0,"show_popup":false}}"#,
        )
        .unwrap();
        assert_eq!(state, AuthorizationState::WaitRegistration);
    }

    #[test]
    fn a_password_state_and_a_reset() {
        let state: PasswordState = serde_json::from_str(
            r#"{"@type":"passwordState","has_password":true,"password_hint":"bird","has_recovery_email_address":true,"has_passport_data":false,"recovery_email_address_code_info":{"@type":"emailAddressAuthenticationCodeInfo","email_address_pattern":"n**@example.com","length":6},"login_email_address_pattern":"","pending_reset_date":0}"#,
        )
        .unwrap();
        assert!(state.has_password && state.has_recovery_email_address);
        assert_eq!(state.recovery_email_address_code_info.map(|info| info.length), Some(6));
        let state: PasswordState = serde_json::from_str(
            r#"{"@type":"passwordState","has_password":false,"password_hint":"","has_recovery_email_address":false,"has_passport_data":false,"recovery_email_address_code_info":null,"login_email_address_pattern":"","pending_reset_date":1791387600}"#,
        )
        .unwrap();
        assert_eq!(state.pending_reset_date, 1_791_387_600);
        let result: ResetPasswordResult =
            serde_json::from_str(r#"{"@type":"resetPasswordResultPending","pending_reset_date":1791387600}"#).unwrap();
        assert_eq!(result, ResetPasswordResult::Pending { pending_reset_date: 1_791_387_600 });
    }

    #[test]
    fn int64_values_are_read_from_strings() {
        let value: OptionValue =
            serde_json::from_str(r#"{"@type":"optionValueInteger","value":"9007199254740993"}"#).unwrap();
        assert_eq!(value, OptionValue::Integer { value: 9_007_199_254_740_993 });
    }

    #[test]
    fn a_chat_position_reads_its_order() {
        let update: Update = serde_json::from_str(
            r#"{"@type":"updateChatPosition","chat_id":-1001234567890,"position":{"@type":"chatPosition","list":{"@type":"chatListFolder","chat_folder_id":3},"order":"9221294780217032704","is_pinned":true,"source":null}}"#,
        )
        .unwrap();
        let Update::ChatPosition { chat_id, position } = update else { panic!("not a chat position") };
        assert_eq!(chat_id, -1_001_234_567_890);
        assert_eq!(position.list, ChatList::Folder { chat_folder_id: 3 });
        assert_eq!(position.order, 9_221_294_780_217_032_704);
        assert!(position.is_pinned);
    }

    #[test]
    fn unknown_content_is_other_and_known_content_keeps_its_words() {
        let message: MessageContent =
            serde_json::from_str(r#"{"@type":"messageGiveaway","parameters":{},"winner_count":3}"#).unwrap();
        assert_eq!(message, MessageContent::Other);
        let message: MessageContent = serde_json::from_str(
            r#"{"@type":"messagePhoto","photo":{"@type":"photo","sizes":[]},"caption":{"@type":"formattedText","text":"the view","entities":[]},"has_spoiler":false}"#,
        )
        .unwrap();
        assert_eq!(
            message,
            MessageContent::Photo {
                photo: Photo { minithumbnail: None, sizes: Vec::new() },
                caption: FormattedText { text: "the view".into(), entities: Vec::new() },
                is_secret: false,
            }
        );
    }

    #[test]
    fn self_destructing_content_says_so_and_expired_content_is_known() {
        let photo: MessageContent = serde_json::from_str(
            r#"{"@type":"messagePhoto","photo":{"@type":"photo","sizes":[]},"caption":{"@type":"formattedText","text":"","entities":[]},"has_spoiler":false,"is_secret":true}"#,
        )
        .unwrap();
        assert!(matches!(photo, MessageContent::Photo { is_secret: true, .. }));
        let expired: MessageContent = serde_json::from_str(r#"{"@type":"messageExpiredPhoto"}"#).unwrap();
        assert_eq!(expired, MessageContent::ExpiredPhoto {});
        let message: Message = serde_json::from_str(
            r#"{"@type":"message","id":42,"sender_id":{"@type":"messageSenderUser","user_id":5},"chat_id":5,"is_outgoing":false,"date":1,"edit_date":0,"content":{"@type":"messageExpiredPhoto"},"self_destruct_type":{"@type":"messageSelfDestructTypeTimer","self_destruct_time":10}}"#,
        )
        .unwrap();
        assert_eq!(message.self_destruct_type, Some(MessageSelfDestructType::Timer { self_destruct_time: 10 }));
    }

    #[test]
    fn a_photo_a_video_and_a_sticker_say_what_to_download() {
        let file = |id: i32, done: bool| {
            format!(
                r#"{{"@type":"file","id":{id},"size":48213,"expected_size":48213,"local":{{"@type":"localFile","path":"{}","can_be_downloaded":true,"can_be_deleted":{done},"is_downloading_active":false,"is_downloading_completed":{done},"download_offset":0,"downloaded_prefix_size":0,"downloaded_size":0}},"remote":{{"@type":"remoteFile","id":"AgAC","unique_id":"AQAD","is_uploading_active":false,"is_uploading_completed":true,"uploaded_size":48213}}}}"#,
                if done { "/tdlib/photos/file_7.jpg" } else { "" }
            )
        };
        let photo: MessageContent = serde_json::from_str(&format!(
            r#"{{"@type":"messagePhoto","photo":{{"@type":"photo","has_stickers":false,"minithumbnail":{{"@type":"minithumbnail","width":40,"height":30,"data":"/9j/4AAQ"}},"sizes":[{{"@type":"photoSize","type":"m","photo":{},"width":320,"height":240,"progressive_sizes":[]}},{{"@type":"photoSize","type":"x","photo":{},"width":800,"height":600,"progressive_sizes":[]}}]}},"video":null,"caption":{{"@type":"formattedText","text":"","entities":[]}},"show_caption_above_media":false,"has_spoiler":false,"is_secret":false}}"#,
            file(6, false),
            file(7, true)
        ))
        .unwrap();
        let MessageContent::Photo { photo, .. } = photo else { panic!("not a photo") };
        assert_eq!(photo.minithumbnail.map(|mini| (mini.width, mini.height)), Some((40, 30)));
        assert_eq!(photo.sizes[1].photo.local.path, "/tdlib/photos/file_7.jpg");
        assert!(photo.sizes[1].photo.local.is_downloading_completed && !photo.sizes[0].photo.local.is_downloading_completed);

        let video: MessageContent = serde_json::from_str(&format!(
            r#"{{"@type":"messageVideo","video":{{"@type":"video","duration":42,"width":1280,"height":720,"file_name":"clip.mp4","mime_type":"video/mp4","has_stickers":false,"supports_streaming":true,"minithumbnail":null,"thumbnail":{{"@type":"thumbnail","format":{{"@type":"thumbnailFormatJpeg"}},"width":320,"height":180,"file":{}}},"video":{}}},"alternative_videos":[],"storyboards":[],"cover":null,"start_timestamp":0,"caption":{{"@type":"formattedText","text":"42 seconds","entities":[]}},"show_caption_above_media":false,"has_spoiler":false,"is_secret":false}}"#,
            file(8, false),
            file(9, false)
        ))
        .unwrap();
        let MessageContent::Video { video, caption, .. } = video else { panic!("not a video") };
        assert_eq!((video.duration, video.width, video.height, caption.text.as_str()), (42, 1280, 720, "42 seconds"));
        assert_eq!(video.thumbnail.map(|thumbnail| (thumbnail.format, thumbnail.file.id)), Some((ThumbnailFormat::Jpeg, 8)));

        let sticker: MessageContent = serde_json::from_str(&format!(
            r#"{{"@type":"messageSticker","sticker":{{"@type":"sticker","id":"5368324170671202286","set_id":"5368324170671202049","width":512,"height":512,"emoji":"😀","format":{{"@type":"stickerFormatTgs"}},"full_type":{{"@type":"stickerFullTypeRegular","premium_animation":null}},"thumbnail":{{"@type":"thumbnail","format":{{"@type":"thumbnailFormatWebp"}},"width":128,"height":128,"file":{}}},"sticker":{}}},"is_premium":false}}"#,
            file(10, false),
            file(11, false)
        ))
        .unwrap();
        let MessageContent::Sticker { sticker } = sticker else { panic!("not a sticker") };
        assert_eq!((sticker.width, sticker.height, sticker.emoji.as_str(), sticker.format), (512, 512, "😀", StickerFormat::Tgs));
        assert_eq!(sticker.thumbnail.map(|thumbnail| (thumbnail.format, thumbnail.file.id)), Some((ThumbnailFormat::Webp, 10)));
        assert_eq!(sticker.sticker.id, 11);
    }

    #[test]
    fn a_new_message_without_a_sending_state_is_read() {
        let update: Update = serde_json::from_str(
            r#"{"@type":"updateNewMessage","message":{"@type":"message","id":1048576,"sender_id":{"@type":"messageSenderUser","user_id":42},"chat_id":42,"is_outgoing":false,"date":1790000000,"edit_date":0,"content":{"@type":"messageText","text":{"@type":"formattedText","text":"hi","entities":[]}}}}"#,
        )
        .unwrap();
        let Update::NewMessage { message } = update else { panic!("not a new message") };
        assert_eq!(message.id, 1_048_576);
        assert_eq!(message.sender_id, MessageSender::User { user_id: 42 });
        assert!(message.sending_state.is_none());
    }

    #[test]
    fn a_bot_is_read_although_its_type_has_fields() {
        let update: Update = serde_json::from_str(
            r#"{"@type":"updateUser","user":{"@type":"user","id":93372553,"first_name":"BotFather","last_name":"","usernames":{"@type":"usernames","active_usernames":["BotFather"],"disabled_usernames":[],"editable_username":"BotFather","collectible_usernames":[]},"phone_number":"","status":{"@type":"userStatusEmpty"},"accent_color_id":2,"background_custom_emoji_id":"0","profile_accent_color_id":-1,"profile_background_custom_emoji_id":"0","is_contact":false,"is_mutual_contact":false,"is_close_friend":false,"verification_status":{"@type":"verificationStatus","is_verified":true,"is_scam":false,"is_fake":false,"bot_verification_icon_custom_emoji_id":"0"},"is_premium":false,"is_support":false,"restricts_new_chats":false,"paid_message_star_count":0,"have_access":true,"type":{"@type":"userTypeBot","can_be_edited":false,"can_join_groups":true,"can_read_all_group_messages":false,"has_main_web_app":false,"is_inline":false,"inline_query_placeholder":"","need_location":false,"can_connect_to_business":false,"can_be_added_to_attachment_menu":false,"active_user_count":0},"language_code":"","added_to_attachment_menu":false}}"#,
        )
        .unwrap();
        let Update::User { user } = update else { panic!("not a user") };
        assert_eq!(user.kind, UserType::Bot);
        assert!(user.verification_status.is_some_and(|status| status.is_verified));
        assert_eq!(user.usernames.map(|names| names.active_usernames), Some(vec!["BotFather".to_string()]));
        assert!(user.profile_photo.is_none(), "a user without a photo");
    }

    #[test]
    fn a_chat_photo_is_read_in_its_two_sizes_and_taken_away() {
        let update: Update = serde_json::from_str(
            r#"{"@type":"updateChatPhoto","chat_id":-1001234567890,"photo":{"@type":"chatPhotoInfo","small":{"@type":"file","id":31,"size":4096,"expected_size":4096,"local":{"@type":"localFile","path":"","can_be_downloaded":true,"can_be_deleted":false,"is_downloading_active":false,"is_downloading_completed":false,"download_offset":0,"downloaded_prefix_size":0,"downloaded_size":0},"remote":{"@type":"remoteFile","id":"AQAD","unique_id":"AQAD","is_uploading_active":false,"is_uploading_completed":true,"uploaded_size":4096}},"big":{"@type":"file","id":32,"size":40960,"expected_size":40960,"local":{"@type":"localFile","path":"","can_be_downloaded":true,"can_be_deleted":false,"is_downloading_active":false,"is_downloading_completed":false,"download_offset":0,"downloaded_prefix_size":0,"downloaded_size":0},"remote":{"@type":"remoteFile","id":"AQAE","unique_id":"AQAE","is_uploading_active":false,"is_uploading_completed":true,"uploaded_size":40960}},"minithumbnail":{"@type":"minithumbnail","width":8,"height":8,"data":"/9j/4AAQ"},"has_animation":false,"is_personal":false}}"#,
        )
        .unwrap();
        let Update::ChatPhoto { chat_id, photo: Some(photo) } = update else { panic!("not a chat photo") };
        assert_eq!(chat_id, -1_001_234_567_890);
        assert_eq!((photo.small.id, photo.big.id), (31, 32));
        assert_eq!(photo.minithumbnail.map(|preview| (preview.width, preview.height)), Some((8, 8)));

        let update: Update = serde_json::from_str(r#"{"@type":"updateChatPhoto","chat_id":42,"photo":null}"#).unwrap();
        assert!(matches!(update, Update::ChatPhoto { chat_id: 42, photo: None }));

        // A person's photo has the same shape, and an id of its own that we leave aside.
        let user: User = serde_json::from_str(
            r#"{"@type":"user","id":7,"first_name":"Lin","last_name":"Xia","phone_number":"","profile_photo":{"@type":"profilePhoto","id":"123456789","small":{"@type":"file","id":41,"size":0,"local":{"@type":"localFile","path":"/tdlib/profile_photos/41.jpg","is_downloading_completed":true}},"big":{"@type":"file","id":42,"size":0,"local":{"@type":"localFile","path":"","is_downloading_completed":false}},"minithumbnail":null,"has_animation":false,"is_personal":false},"status":{"@type":"userStatusRecently"},"type":{"@type":"userTypeRegular"}}"#,
        )
        .unwrap();
        let photo = user.profile_photo.expect("a photo");
        assert_eq!(photo.small.id, 41);
        assert!(photo.small.local.is_downloading_completed && photo.minithumbnail.is_none());
    }

    #[test]
    fn a_new_chat_is_read_with_the_fields_we_use() {
        let update: Update = serde_json::from_str(
            r#"{"@type":"updateNewChat","chat":{"@type":"chat","id":-1001234567890,"type":{"@type":"chatTypeSupergroup","supergroup_id":1234567890,"is_channel":true},"title":"Tech Morning","photo":null,"accent_color_id":0,"background_custom_emoji_id":"0","profile_accent_color_id":-1,"profile_background_custom_emoji_id":"0","permissions":{"@type":"chatPermissions","can_send_basic_messages":false,"can_send_audios":false,"can_send_documents":false,"can_send_photos":false,"can_send_videos":false,"can_send_video_notes":false,"can_send_voice_notes":false,"can_send_polls":false,"can_send_other_messages":false,"can_add_link_previews":false,"can_react_to_messages":true,"can_change_info":false,"can_invite_users":false,"can_pin_messages":false,"can_create_topics":false},"positions":[],"chat_lists":[],"has_protected_content":false,"is_translatable":false,"is_marked_as_unread":false,"view_as_topics":false,"has_scheduled_messages":false,"can_be_deleted_only_for_self":true,"can_be_deleted_for_all_users":false,"can_be_reported":true,"default_disable_notification":false,"unread_count":12,"last_read_inbox_message_id":1048576,"last_read_outbox_message_id":0,"unread_mention_count":0,"unread_reaction_count":0,"notification_settings":{"@type":"chatNotificationSettings","use_default_mute_for":true,"mute_for":0,"use_default_sound":true,"sound_id":"0","use_default_show_preview":true,"show_preview":false,"use_default_mute_stories":true,"mute_stories":false,"use_default_story_sound":true,"story_sound_id":"0","use_default_show_story_poster":true,"show_story_poster":false,"use_default_disable_pinned_message_notifications":true,"disable_pinned_message_notifications":false,"use_default_disable_mention_notifications":true,"disable_mention_notifications":false},"available_reactions":{"@type":"chatAvailableReactionsSome","reactions":[],"max_reaction_count":11},"message_auto_delete_time":0,"emoji_status":null,"background":null,"theme":null,"action_bar":null,"business_bot_manage_bar":null,"video_chat":{"@type":"videoChat","group_call_id":0,"has_participants":false,"default_participant_id":null},"pending_join_requests":null,"reply_markup_message_id":0,"draft_message":null,"client_data":""}}"#,
        )
        .unwrap();
        let Update::NewChat { chat } = update else { panic!("not a chat") };
        assert_eq!(chat.kind, ChatType::Supergroup { supergroup_id: 1_234_567_890, is_channel: true });
        assert_eq!(chat.unread_count, 12);
        assert!(chat.last_message.is_none());
        assert!(chat.notification_settings.use_default_mute_for);
    }

    #[test]
    fn a_login_code_says_where_it_went_and_how_long_it_is() {
        let state: AuthorizationState = serde_json::from_str(
            r#"{"@type":"authorizationStateWaitCode","code_info":{"@type":"authenticationCodeInfo","phone_number":"+8613800132046","type":{"@type":"authenticationCodeTypeTelegramMessage","length":5},"next_type":{"@type":"authenticationCodeTypeSms","length":5},"timeout":0}}"#,
        )
        .unwrap();
        let AuthorizationState::WaitCode { code_info } = state else { panic!("not waiting for a code") };
        assert_eq!(code_info.kind, AuthenticationCodeType::TelegramMessage { length: 5 });
        assert!(code_info.next_type.is_some());
        let state: AuthorizationState = serde_json::from_str(
            r#"{"@type":"authorizationStateWaitCode","code_info":{"@type":"authenticationCodeInfo","phone_number":"+8613800132046","type":{"@type":"authenticationCodeTypeFirebaseIos","receipt":"","push_timeout":0,"length":6},"next_type":null,"timeout":0}}"#,
        )
        .unwrap();
        let AuthorizationState::WaitCode { code_info } = state else { panic!("not waiting for a code") };
        assert_eq!(code_info.kind, AuthenticationCodeType::Other);
        assert!(code_info.next_type.is_none());
    }

    #[test]
    fn a_history_may_have_holes() {
        let messages: Messages = serde_json::from_str(
            r#"{"@type":"messages","total_count":2,"messages":[{"@type":"message","id":2097152,"sender_id":{"@type":"messageSenderChat","chat_id":-1001234567890},"chat_id":-1001234567890,"sending_state":null,"is_outgoing":false,"date":1790000000,"edit_date":0,"content":{"@type":"messageText","text":{"@type":"formattedText","text":"post","entities":[]}}},null]}"#,
        )
        .unwrap();
        assert_eq!(messages.messages.len(), 2);
        assert!(messages.messages[1].is_none());
    }

    #[test]
    fn a_failed_message_can_be_retried() {
        let update: Update = serde_json::from_str(
            r#"{"@type":"updateMessageSendFailed","message":{"@type":"message","id":3145728,"sender_id":{"@type":"messageSenderUser","user_id":1},"chat_id":42,"sending_state":{"@type":"messageSendingStateFailed","error":{"@type":"error","code":400,"message":"CHAT_WRITE_FORBIDDEN"},"can_retry":true,"need_another_sender":false,"need_another_reply_quote":false,"need_drop_reply":false,"required_paid_message_star_count":0,"retry_after":0.0},"is_outgoing":true,"date":1790000000,"edit_date":0,"content":{"@type":"messageText","text":{"@type":"formattedText","text":"hi","entities":[]}}},"old_message_id":1,"error":{"@type":"error","code":400,"message":"CHAT_WRITE_FORBIDDEN"}}"#,
        )
        .unwrap();
        let Update::MessageSendFailed { message, old_message_id } = update else { panic!("not a failure") };
        assert_eq!(old_message_id, 1);
        assert_eq!(message.sending_state, Some(MessageSendingState::Failed { can_retry: true }));
    }

    #[test]
    fn a_reply_and_a_forward_say_where_they_come_from() {
        let message: Message = serde_json::from_str(
            r#"{"@type":"message","id":5242880,"sender_id":{"@type":"messageSenderUser","user_id":7},"chat_id":-1001234567890,"is_outgoing":false,"date":1790000000,"edit_date":0,"forward_info":{"@type":"messageForwardInfo","origin":{"@type":"messageOriginChannel","chat_id":-1009876543210,"message_id":42,"author_signature":""},"date":1789990000,"source":null,"public_service_announcement_type":""},"reply_to":{"@type":"messageReplyToMessage","chat_id":-1001234567890,"message_id":4194304,"quote":null,"checklist_task_id":0,"poll_option_id":"","origin":null,"origin_send_date":0,"content":null},"media_album_id":"0","content":{"@type":"messageText","text":{"@type":"formattedText","text":"ok","entities":[]}}}"#,
        )
        .unwrap();
        assert_eq!(message.forward_info.map(|info| info.origin), Some(MessageOrigin::Channel { chat_id: -1_009_876_543_210 }));
        let Some(MessageReplyTo::Message { chat_id, message_id, origin, quote, .. }) = message.reply_to else { panic!("not a reply") };
        assert_eq!((chat_id, message_id), (-1_001_234_567_890, 4_194_304));
        assert!(origin.is_none() && quote.is_none());
        let story: MessageReplyTo =
            serde_json::from_str(r#"{"@type":"messageReplyToStory","story_poster_chat_id":7,"story_id":3}"#).unwrap();
        assert_eq!(story, MessageReplyTo::Other);
    }

    #[test]
    fn a_report_is_led_by_the_server() {
        let result: ReportChatResult = serde_json::from_str(
            r#"{"@type":"reportChatResultOptionRequired","title":"Report","options":[{"@type":"reportOption","id":"AQ==","text":"Spam"}]}"#,
        )
        .unwrap();
        assert_eq!(
            result,
            ReportChatResult::OptionRequired { title: "Report".into(), options: vec![ReportOption { id: "AQ==".into(), text: "Spam".into() }] }
        );
        let result: ReportChatResult =
            serde_json::from_str(r#"{"@type":"reportChatResultTextRequired","option_id":"Ag==","is_optional":true}"#).unwrap();
        assert_eq!(result, ReportChatResult::TextRequired { option_id: "Ag==".into(), is_optional: true });
        let properties: MessageProperties = serde_json::from_str(
            r#"{"@type":"messageProperties","can_add_offer":false,"can_be_copied":true,"can_be_deleted_only_for_self":false,"can_be_deleted_for_all_users":true,"can_be_edited":true,"can_be_forwarded":true,"can_be_replied":true,"can_be_saved":true,"can_get_link":true,"can_report_chat":false}"#,
        )
        .unwrap();
        assert!(properties.can_be_edited && properties.can_get_link && !properties.can_report_chat);
    }

    #[test]
    fn muting_sends_every_setting_back() {
        let settings: ChatNotificationSettings = serde_json::from_str(
            r#"{"@type":"chatNotificationSettings","use_default_mute_for":true,"mute_for":0,"use_default_sound":false,"sound_id":"5037419087648243712","use_default_show_preview":true,"show_preview":false}"#,
        )
        .unwrap();
        let muted = settings.with_mute_for(i32::MAX);
        assert_eq!(muted["use_default_mute_for"], false);
        assert_eq!(muted["mute_for"], i32::MAX);
        assert_eq!(muted["use_default_sound"], false);
        assert_eq!(muted["sound_id"], "5037419087648243712");
    }
}
