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
    ChatDraftMessage { chat_id: i64, positions: Vec<ChatPosition> },
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
    #[serde(rename = "updateMessageEdited")]
    MessageEdited { chat_id: i64, message_id: i64, edit_date: i32 },
    #[serde(rename = "updateDeleteMessages")]
    DeleteMessages { chat_id: i64, message_ids: Vec<i64>, is_permanent: bool },

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
    pub status: UserStatus,
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
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SupergroupFullInfo {
    pub member_count: i32,
}

/// What we are in a group or channel: who may write where.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum ChatMemberStatus {
    #[serde(rename = "chatMemberStatusCreator")]
    Creator,
    #[serde(rename = "chatMemberStatusAdministrator")]
    Administrator { rights: AdministratorRights },
    #[serde(rename = "chatMemberStatusMember")]
    Member,
    #[serde(rename = "chatMemberStatusRestricted")]
    Restricted { is_member: bool, permissions: ChatPermissions },
    #[serde(rename = "chatMemberStatusLeft")]
    Left,
    #[serde(rename = "chatMemberStatusBanned")]
    Banned,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AdministratorRights {
    pub can_post_messages: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ChatPermissions {
    pub can_send_basic_messages: bool,
}

// ---- chats ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: ChatType,
    pub title: String,
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScopeNotificationSettings {
    pub mute_for: i32,
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
    pub content: MessageContent,
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
}

/// What a message holds. Kinds FinchGram does not show yet are `Other`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "@type")]
pub enum MessageContent {
    #[serde(rename = "messageText")]
    Text { text: FormattedText },
    #[serde(rename = "messagePhoto")]
    Photo { caption: FormattedText },
    #[serde(rename = "messageVideo")]
    Video { caption: FormattedText },
    #[serde(rename = "messageAnimation")]
    Animation { caption: FormattedText },
    #[serde(rename = "messageAudio")]
    Audio { audio: Audio, caption: FormattedText },
    #[serde(rename = "messageDocument")]
    Document { document: Document, caption: FormattedText },
    #[serde(rename = "messageVoiceNote")]
    VoiceNote { caption: FormattedText },
    #[serde(rename = "messageVideoNote")]
    VideoNote {},
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Audio {
    pub title: String,
    pub performer: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Document {
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Sticker {
    pub emoji: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Venue {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Contact {
    pub first_name: String,
    pub last_name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Poll {
    pub question: FormattedText,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Game {
    pub title: String,
}

/// getChatHistory's answer. TDLib may leave out messages it no longer has.
#[derive(Debug, Clone, Deserialize)]
pub struct Messages {
    pub messages: Vec<Option<Message>>,
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
        assert_eq!(message, MessageContent::Photo { caption: FormattedText { text: "the view".into() } });
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
