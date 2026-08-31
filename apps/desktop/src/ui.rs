use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use chrono::Utc;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _,
    PathPromptOptions, Render, Styled as _, Subscription, Timer, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Theme, ThemeMode,
    avatar::Avatar,
    badge::Badge,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use iamrust_client_core::{CacheStats, LocalStore};
use iamrust_domain::{
    Conversation, ConversationId, ConversationKind, EventKind, MemberRole, Message, MessageContent,
    MessageId, MessageStatus, UserId,
};
use iamrust_protocol::{
    BootstrapResponse, CallSignal, CreateGroupPollRequest, DeviceInfo, GroupAnnouncement,
    GroupFileItem, GroupJoinRequest, GroupJoinRequestStatus, GroupPoll, MessageDetails,
    QrLoginStartResponse, SecondFactorSetupResponse, SecondFactorStatus, SendMessageRequest,
    UpdateConversationSettingsRequest,
};
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::{
    api::{ApiClient, ClientError},
    desktop,
    model::{
        AuthMode, ChatMessage, ConversationPreview, Navigation, conversations_from_bootstrap,
        demo_conversations, demo_messages, messages_from_domain,
    },
    realtime::{self, ConnectionState, RealtimeEvent, RealtimeHandle},
    shell::{WINDOW_PLACEMENT_SETTING, WindowPlacement},
};

#[allow(clippy::struct_excessive_bools)]
pub struct IamRustApp {
    authenticated: bool,
    navigation: Navigation,
    selected_conversation: usize,
    account: Entity<InputState>,
    register_email: Entity<InputState>,
    register_username: Entity<InputState>,
    register_nickname: Entity<InputState>,
    password: Entity<InputState>,
    second_factor_code: Entity<InputState>,
    reset_email: Entity<InputState>,
    reset_token: Entity<InputState>,
    new_password: Entity<InputState>,
    search: Entity<InputState>,
    local_message_search: Entity<InputState>,
    composer: Entity<InputState>,
    contact_filter: Entity<InputState>,
    user_search: Entity<InputState>,
    friend_request_message: Entity<InputState>,
    profile_nickname: Entity<InputState>,
    profile_signature: Entity<InputState>,
    group_name: Entity<InputState>,
    group_edit_name: Entity<InputState>,
    group_announcement_input: Entity<InputState>,
    group_poll_question: Entity<InputState>,
    group_poll_option_a: Entity<InputState>,
    group_poll_option_b: Entity<InputState>,
    security_current_password: Entity<InputState>,
    second_factor_password: Entity<InputState>,
    security_new_password: Entity<InputState>,
    security_confirm_password: Entity<InputState>,
    security_code: Entity<InputState>,
    qr_approval_payload: Entity<InputState>,
    conversations: Vec<ConversationPreview>,
    messages: Vec<ChatMessage>,
    api: Option<Arc<ApiClient>>,
    auth_busy: bool,
    auth_error: Option<String>,
    auth_notice: Option<String>,
    profile_name: String,
    auth_mode: AuthMode,
    runtime: Arc<Runtime>,
    store: Option<LocalStore>,
    cache_error: Option<String>,
    offline: bool,
    bootstrap: Option<BootstrapResponse>,
    messages_loading: bool,
    timeline_loading_older: bool,
    timeline_next_cursor: Option<u64>,
    timeline_generation: u64,
    message_error: Option<String>,
    message_action_busy: Option<MessageId>,
    message_confirmation: Option<MessageConfirmation>,
    reply_target: Option<ReplyTarget>,
    forward_message_id: Option<MessageId>,
    message_details: HashMap<MessageId, MessageDetails>,
    message_details_open: Option<MessageId>,
    draft_revision: u64,
    typing_revision: u64,
    connection_state: ConnectionState,
    realtime: Option<RealtimeHandle>,
    realtime_generation: u64,
    typing_users: HashMap<ConversationId, HashSet<iamrust_domain::UserId>>,
    incoming_call: Option<(ConversationId, Uuid, bool)>,
    pending_upload: Option<PendingUpload>,
    transfer_busy: bool,
    outbox_flushing: bool,
    outbox_flush_generation: u64,
    selected_contact: usize,
    user_search_results: Vec<iamrust_domain::UserProfile>,
    local_search_results: Vec<Message>,
    local_search_busy: bool,
    local_search_error: Option<String>,
    action_busy: bool,
    action_error: Option<String>,
    delete_friend_confirmation: Option<iamrust_domain::UserId>,
    selected_group_members: HashSet<iamrust_domain::UserId>,
    creating_group: bool,
    group_details_open: bool,
    group_details_loading: bool,
    group_mute_all: bool,
    group_announcements: Vec<GroupAnnouncement>,
    group_files: Vec<GroupFileItem>,
    group_join_requests: Vec<GroupJoinRequest>,
    group_polls: Vec<GroupPoll>,
    group_invite_members: HashSet<UserId>,
    selected_poll_options: HashSet<Uuid>,
    group_confirmation: Option<GroupConfirmation>,
    cache_stats: Option<CacheStats>,
    cache_encryption: Option<bool>,
    retain_cache_on_logout: bool,
    notifications_enabled: bool,
    sounds_enabled: bool,
    privacy_mode: bool,
    window_active: bool,
    theme_preference: String,
    window_placement_revision: Arc<AtomicU64>,
    clear_cache_confirmation: bool,
    devices: Vec<DeviceInfo>,
    security_loading: bool,
    security_loaded: bool,
    second_factor_status: Option<SecondFactorStatus>,
    second_factor_setup: Option<SecondFactorSetupResponse>,
    recovery_codes: Vec<String>,
    revoke_device_confirmation: Option<iamrust_domain::DeviceId>,
    qr_challenge: Option<QrLoginStartResponse>,
    qr_generation: u64,
    _subscriptions: Vec<Subscription>,
}

enum RestoreOutcome {
    Live(BootstrapResponse),
    Cached(BootstrapResponse, String),
    LoggedOut,
    Failed(String),
}

enum TimelineOutcome {
    Live(Vec<Message>, Option<u64>, Option<String>),
    Cached(Vec<Message>, String),
    Failed(String),
}

struct GroupDetailsData {
    mute_all: bool,
    announcements: Vec<GroupAnnouncement>,
    files: Vec<GroupFileItem>,
    join_requests: Vec<GroupJoinRequest>,
    polls: Vec<GroupPoll>,
}

struct OutboxFlushReport {
    sent: Vec<(MessageId, MessageId)>,
    retrying: Vec<(MessageId, String)>,
    cache_warning: Option<String>,
}

#[derive(Clone, Debug)]
struct ReplyTarget {
    message_id: MessageId,
    author: String,
    body: String,
}

struct MessageActionContext {
    index: usize,
    retry_id: Option<MessageId>,
    server_id: Option<MessageId>,
    recallable: bool,
    author: String,
    body: String,
}

#[derive(Clone)]
struct PendingUpload {
    path: PathBuf,
    file_name: String,
    byte_size: u64,
    image: bool,
}

struct AttachmentSendContext {
    upload: PendingUpload,
    client_message_id: MessageId,
    conversation_id: ConversationId,
    sender_id: UserId,
    reply_to: Option<MessageId>,
}

enum AttachmentUploadOutcome {
    Completed {
        label: String,
        attachment: iamrust_domain::Attachment,
        send: SendOutcome,
    },
    Failed {
        client_message_id: MessageId,
        error: String,
    },
}

enum SendOutcome {
    Sent {
        client_message_id: MessageId,
        message_id: MessageId,
        cache_warning: Option<String>,
    },
    Failed {
        client_message_id: MessageId,
        error: String,
        cache_warning: Option<String>,
    },
}

enum AuthOutcome {
    Authenticated(Box<BootstrapResponse>),
    ResetRequested,
    ResetConfirmed,
    QrStarted(QrLoginStartResponse),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupConfirmation {
    Leave,
    Disband,
    Remove(UserId),
    Transfer(UserId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageConfirmation {
    Recall(MessageId),
    Discard(MessageId),
}

impl IamRustApp {
    #[allow(clippy::too_many_lines)]
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        runtime: Arc<Runtime>,
        store: Option<LocalStore>,
        cache_error: Option<String>,
        theme_preference: String,
    ) -> Self {
        let account = cx.new(|cx| InputState::new(window, cx).placeholder("用户名或邮箱"));
        let register_email = cx.new(|cx| InputState::new(window, cx).placeholder("邮箱"));
        let register_username = cx.new(|cx| InputState::new(window, cx).placeholder("用户名"));
        let register_nickname = cx.new(|cx| InputState::new(window, cx).placeholder("昵称"));
        let password = cx.new(|cx| InputState::new(window, cx).placeholder("密码").masked(true));
        let second_factor_code =
            cx.new(|cx| InputState::new(window, cx).placeholder("双因素验证码（如已启用）"));
        let reset_email = cx.new(|cx| InputState::new(window, cx).placeholder("注册邮箱"));
        let reset_token = cx.new(|cx| InputState::new(window, cx).placeholder("邮件中的重置令牌"));
        let new_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("新密码")
                .masked(true)
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索会话"));
        let local_message_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索本机已同步消息"));
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 6)
                .placeholder("输入消息，Enter 发送")
        });
        let contact_filter = cx.new(|cx| InputState::new(window, cx).placeholder("筛选联系人"));
        let user_search = cx.new(|cx| InputState::new(window, cx).placeholder("输入精确用户名"));
        let friend_request_message = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("好友验证消息（可选）")
                .auto_grow(1, 3)
        });
        let profile_nickname = cx.new(|cx| InputState::new(window, cx).placeholder("新的昵称"));
        let profile_signature =
            cx.new(|cx| InputState::new(window, cx).placeholder("新的个性签名"));
        let group_name = cx.new(|cx| InputState::new(window, cx).placeholder("群名称"));
        let group_edit_name = cx.new(|cx| InputState::new(window, cx).placeholder("新的群名称"));
        let group_announcement_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("发布群公告")
                .auto_grow(1, 4)
        });
        let group_poll_question = cx.new(|cx| InputState::new(window, cx).placeholder("投票问题"));
        let group_poll_option_a = cx.new(|cx| InputState::new(window, cx).placeholder("选项 A"));
        let group_poll_option_b = cx.new(|cx| InputState::new(window, cx).placeholder("选项 B"));
        let security_current_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("当前密码")
                .masked(true)
        });
        let second_factor_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("当前密码")
                .masked(true)
        });
        let security_new_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("新密码（至少 10 位）")
                .masked(true)
        });
        let security_confirm_password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("再次输入新密码")
                .masked(true)
        });
        let security_code = cx.new(|cx| InputState::new(window, cx).placeholder("验证码或恢复码"));
        let qr_approval_payload =
            cx.new(|cx| InputState::new(window, cx).placeholder("iamrust://auth/qr-login?…"));
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.schedule_draft_save(cx);
                    this.schedule_typing_stop(cx);
                }
                InputEvent::PressEnter { secondary: false } => {
                    this.send_composer(window, cx);
                }
                _ => {}
            },
        );
        let contact_filter_subscription =
            cx.subscribe(&contact_filter, |this, _, _: &InputEvent, cx| {
                this.selected_contact = 0;
                cx.notify();
            });
        let user_search_subscription =
            cx.subscribe(&user_search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.start_user_search(cx);
                }
            });
        let conversation_search_subscription =
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify());
        let local_message_search_subscription =
            cx.subscribe(&local_message_search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.start_local_message_search(cx);
                }
            });
        let window_bounds_subscription = cx.observe_window_bounds(window, |this, window, cx| {
            this.schedule_window_placement_save(window, cx);
        });
        let window_activation_subscription =
            cx.observe_window_activation(window, |this, window, _cx| {
                this.window_active = window.is_window_active();
            });
        let window_appearance_subscription =
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.theme_preference == "system" {
                    Theme::change(window.appearance(), Some(window), cx);
                    cx.notify();
                }
            });

        let (api, auth_error) = match ApiClient::from_environment() {
            Ok(api) => (Some(Arc::new(api)), None),
            Err(error) => (None, Some(error.user_message())),
        };
        let mut app = Self {
            authenticated: false,
            navigation: Navigation::Chats,
            selected_conversation: 0,
            account,
            register_email,
            register_username,
            register_nickname,
            password,
            second_factor_code,
            reset_email,
            reset_token,
            new_password,
            search,
            local_message_search,
            composer,
            contact_filter,
            user_search,
            friend_request_message,
            profile_nickname,
            profile_signature,
            group_name,
            group_edit_name,
            group_announcement_input,
            group_poll_question,
            group_poll_option_a,
            group_poll_option_b,
            security_current_password,
            second_factor_password,
            security_new_password,
            security_confirm_password,
            security_code,
            qr_approval_payload,
            conversations: demo_conversations(),
            messages: demo_messages(),
            api,
            auth_busy: false,
            auth_error,
            auth_notice: None,
            profile_name: "我".to_owned(),
            auth_mode: AuthMode::Login,
            runtime,
            store,
            cache_error,
            offline: false,
            bootstrap: None,
            messages_loading: false,
            timeline_loading_older: false,
            timeline_next_cursor: None,
            timeline_generation: 0,
            message_error: None,
            message_action_busy: None,
            message_confirmation: None,
            reply_target: None,
            forward_message_id: None,
            message_details: HashMap::new(),
            message_details_open: None,
            draft_revision: 0,
            typing_revision: 0,
            connection_state: ConnectionState::Offline,
            realtime: None,
            realtime_generation: 0,
            typing_users: HashMap::new(),
            incoming_call: None,
            pending_upload: None,
            transfer_busy: false,
            outbox_flushing: false,
            outbox_flush_generation: 0,
            selected_contact: 0,
            user_search_results: Vec::new(),
            local_search_results: Vec::new(),
            local_search_busy: false,
            local_search_error: None,
            action_busy: false,
            action_error: None,
            delete_friend_confirmation: None,
            selected_group_members: HashSet::new(),
            creating_group: false,
            group_details_open: false,
            group_details_loading: false,
            group_mute_all: false,
            group_announcements: Vec::new(),
            group_files: Vec::new(),
            group_join_requests: Vec::new(),
            group_polls: Vec::new(),
            group_invite_members: HashSet::new(),
            selected_poll_options: HashSet::new(),
            group_confirmation: None,
            cache_stats: None,
            cache_encryption: None,
            retain_cache_on_logout: true,
            notifications_enabled: true,
            sounds_enabled: true,
            privacy_mode: false,
            window_active: window.is_window_active(),
            theme_preference,
            window_placement_revision: Arc::new(AtomicU64::new(0)),
            clear_cache_confirmation: false,
            devices: Vec::new(),
            security_loading: false,
            security_loaded: false,
            second_factor_status: None,
            second_factor_setup: None,
            recovery_codes: Vec::new(),
            revoke_device_confirmation: None,
            qr_challenge: None,
            qr_generation: 0,
            _subscriptions: vec![
                composer_subscription,
                contact_filter_subscription,
                user_search_subscription,
                conversation_search_subscription,
                local_message_search_subscription,
                window_bounds_subscription,
                window_activation_subscription,
                window_appearance_subscription,
            ],
        };
        app.restore_session(cx);
        app.load_local_preferences(cx);
        app
    }

    fn render_auth(&self, cx: &mut Context<Self>) -> AnyElement {
        let submit = cx.listener(|this, _, _, cx| this.start_authentication(cx));

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_4()
                    .p_8()
                    .rounded_xl()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .child(
                        v_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("I Am Rust"),
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Rust 原生多平台即时通讯"),
                            ),
                    )
                    .child(self.render_auth_fields(cx))
                    .when_some(self.auth_error.clone(), |panel, error| {
                        panel.child(
                            div()
                                .p_3()
                                .rounded_md()
                                .bg(cx.theme().danger.opacity(0.12))
                                .text_color(cx.theme().danger)
                                .text_size(px(12.))
                                .child(error),
                        )
                    })
                    .when_some(self.auth_notice.clone(), |panel, notice| {
                        panel.child(
                            div()
                                .p_3()
                                .rounded_md()
                                .bg(cx.theme().primary.opacity(0.1))
                                .text_color(cx.theme().primary)
                                .text_size(px(12.))
                                .child(notice),
                        )
                    })
                    .child(
                        Button::new("login")
                            .primary()
                            .large()
                            .loading(self.auth_busy)
                            .label(self.auth_submit_label())
                            .on_click(submit),
                    )
                    .child(
                        Button::new("demo")
                            .outline()
                            .large()
                            .label("进入本地演示")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.qr_generation = this.qr_generation.wrapping_add(1);
                                this.qr_challenge = None;
                                this.authenticated = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_center()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child("GPUI + gpui-component · 无 WebView"),
                    ),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)]
    fn render_auth_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("auth-login-mode")
                            .flex_1()
                            .selected(self.auth_mode == AuthMode::Login)
                            .label("登录")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.auth_mode = AuthMode::Login;
                                this.auth_error = None;
                                this.auth_notice = None;
                                this.qr_generation = this.qr_generation.wrapping_add(1);
                                this.qr_challenge = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("auth-register-mode")
                            .flex_1()
                            .selected(self.auth_mode == AuthMode::Register)
                            .label("注册")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.auth_mode = AuthMode::Register;
                                this.auth_error = None;
                                this.auth_notice = None;
                                this.qr_generation = this.qr_generation.wrapping_add(1);
                                this.qr_challenge = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("auth-reset-mode")
                            .flex_1()
                            .ghost()
                            .small()
                            .selected(matches!(
                                self.auth_mode,
                                AuthMode::PasswordReset | AuthMode::PasswordResetConfirm
                            ))
                            .label("找回密码")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.auth_mode = AuthMode::PasswordReset;
                                this.auth_error = None;
                                this.auth_notice = None;
                                this.qr_generation = this.qr_generation.wrapping_add(1);
                                this.qr_challenge = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("auth-qr-mode")
                            .flex_1()
                            .ghost()
                            .small()
                            .selected(self.auth_mode == AuthMode::QrLogin)
                            .label("二维码登录")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.auth_mode = AuthMode::QrLogin;
                                this.auth_error = None;
                                this.auth_notice = None;
                                this.qr_challenge = None;
                                this.qr_generation = this.qr_generation.wrapping_add(1);
                                cx.notify();
                            })),
                    ),
            )
            .when(self.auth_mode == AuthMode::Login, |fields| {
                fields
                    .child(Input::new(&self.account).large().cleanable(true))
                    .child(Input::new(&self.second_factor_code).large().cleanable(true))
            })
            .when(self.auth_mode == AuthMode::Register, |fields| {
                fields
                    .child(Input::new(&self.register_email).large().cleanable(true))
                    .child(Input::new(&self.register_username).large().cleanable(true))
                    .child(Input::new(&self.register_nickname).large().cleanable(true))
            })
            .when(
                matches!(self.auth_mode, AuthMode::Login | AuthMode::Register),
                |fields| fields.child(Input::new(&self.password).large().mask_toggle()),
            )
            .when(self.auth_mode == AuthMode::PasswordReset, |fields| {
                fields
                    .child(Input::new(&self.reset_email).large().cleanable(true))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child("我们会向该邮箱发送一次性重置令牌。"),
                    )
            })
            .when(self.auth_mode == AuthMode::PasswordResetConfirm, |fields| {
                fields
                    .child(Input::new(&self.reset_token).large().cleanable(true))
                    .child(Input::new(&self.new_password).large().mask_toggle())
            })
            .when(self.auth_mode == AuthMode::QrLogin, |fields| {
                if let Some(challenge) = &self.qr_challenge {
                    fields.child(
                        v_flex()
                            .gap_2()
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_center()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("请用已登录设备扫码确认"),
                            )
                            .child(
                                div()
                                    .p_3()
                                    .rounded_md()
                                    .bg(cx.theme().secondary)
                                    .text_size(px(11.))
                                    .child(challenge.qr_payload.clone()),
                            )
                            .child(
                                div()
                                    .text_center()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "有效期至 {}",
                                        challenge.expires_at.format("%H:%M:%S")
                                    )),
                            ),
                    )
                } else {
                    fields.child(
                        div()
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_center()
                            .text_color(cx.theme().muted_foreground)
                            .child("生成二维码后，可在另一台已登录设备上确认登录。"),
                    )
                }
            })
            .into_any_element()
    }

    const fn auth_submit_label(&self) -> &'static str {
        match (self.auth_busy, self.auth_mode) {
            (true, AuthMode::Login) => "登录中…",
            (true, AuthMode::Register) => "注册中…",
            (true, AuthMode::PasswordReset) => "正在发送…",
            (true, AuthMode::PasswordResetConfirm) => "正在重置…",
            (true, AuthMode::QrLogin) => "正在生成…",
            (false, AuthMode::Login) => "登录",
            (false, AuthMode::Register) => "注册并登录",
            (false, AuthMode::PasswordReset) => "发送重置令牌",
            (false, AuthMode::PasswordResetConfirm) => "确认新密码",
            (false, AuthMode::QrLogin) => "生成登录二维码",
        }
    }

    fn render_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let buttons = Navigation::ALL.into_iter().map(|navigation| {
            Button::new(("nav", navigation as usize))
                .ghost()
                .small()
                .selected(self.navigation == navigation)
                .label(navigation.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigation = navigation;
                    if navigation == Navigation::Settings && !this.security_loaded {
                        this.load_security_settings(cx);
                    }
                    cx.notify();
                }))
        });

        v_flex()
            .h_full()
            .w(px(92.))
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .py_4()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                v_flex()
                    .items_center()
                    .gap_4()
                    .child(Avatar::new().name(self.profile_name.clone()).large())
                    .children(buttons),
            )
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("logout")
                            .ghost()
                            .small()
                            .label("退出")
                            .on_click(cx.listener(|this, _, _, cx| this.start_logout(cx))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child("v0.1.0"),
                    ),
            )
            .into_any_element()
    }

    fn render_conversation_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let rows = self
            .conversations
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, conversation)| {
                query.is_empty() || conversation.name.to_lowercase().contains(&query)
            })
            .map(|(index, conversation)| self.render_conversation_row(index, conversation, cx))
            .collect::<Vec<_>>();

        v_flex()
            .h_full()
            .w(px(320.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("会话"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new("mark-all-read")
                                            .small()
                                            .ghost()
                                            .disabled(self.action_busy)
                                            .label("全已读")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.mark_all_conversations_read(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("new-chat")
                                            .small()
                                            .primary()
                                            .label("新建")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.navigation = Navigation::Contacts;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(Input::new(&self.search).small().cleanable(true)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .p_2()
                    .gap_1()
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_conversation_row(
        &self,
        index: usize,
        conversation: ConversationPreview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_conversation == index;
        let name = conversation.name.clone();
        Button::new(("conversation", index))
            .ghost()
            .selected(selected)
            .w_full()
            .h(px(72.))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_conversation(index, window, cx);
            }))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .child(
                        Badge::new()
                            .count(conversation.unread)
                            .child(Avatar::new().name(name)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(conversation.name),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(conversation.timestamp),
                                    ),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(conversation.summary),
                            ),
                    ),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)]
    fn render_chat(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let title = self
            .conversations
            .get(self.selected_conversation)
            .map_or("选择会话", |conversation| conversation.name.as_str());
        let messages = self
            .messages
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, message)| self.render_message(index, message, cx))
            .collect::<Vec<_>>();
        v_flex()
            .h_full()
            .flex_1()
            .min_w_0()
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .h(px(68.))
                    .flex_shrink_0()
                    .justify_between()
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(title.to_owned()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.chat_presence_label()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("voice-call")
                                    .ghost()
                                    .small()
                                    .label("语音")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_call(false, cx);
                                    })),
                            )
                            .child(
                                Button::new("video-call")
                                    .ghost()
                                    .small()
                                    .label("视频")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_call(true, cx);
                                    })),
                            )
                            .child(
                                Button::new("details")
                                    .ghost()
                                    .selected(self.group_details_open)
                                    .label("详情")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_conversation_details(cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_4()
                    .p_5()
                    .when(self.timeline_next_cursor.is_some(), |timeline| {
                        timeline.child(
                            h_flex().justify_center().child(
                                Button::new("load-older-messages")
                                    .small()
                                    .outline()
                                    .loading(self.timeline_loading_older)
                                    .label(if self.timeline_loading_older {
                                        "加载中…"
                                    } else {
                                        "加载更早消息"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.load_older_messages(cx);
                                    })),
                            ),
                        )
                    })
                    .when(self.messages_loading, |timeline| {
                        timeline.child(
                            div()
                                .text_center()
                                .text_color(cx.theme().muted_foreground)
                                .child("正在加载消息…"),
                        )
                    })
                    .when_some(self.message_error.clone(), |timeline, error| {
                        timeline.child(
                            div()
                                .p_3()
                                .rounded_md()
                                .bg(cx.theme().danger.opacity(0.1))
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    })
                    .children(messages),
            )
            .child(self.render_composer(cx))
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)]
    fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let send = cx.listener(|this, _, window, cx| this.send_composer(window, cx));
        v_flex()
            .flex_shrink_0()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(Button::new("emoji").ghost().small().label("Emoji"))
                    .child(
                        Button::new("image")
                            .ghost()
                            .small()
                            .disabled(self.transfer_busy)
                            .label("图片")
                            .on_click(cx.listener(|_, _, _, cx| {
                                Self::choose_attachment(true, cx);
                            })),
                    )
                    .child(
                        Button::new("file")
                            .ghost()
                            .small()
                            .disabled(self.transfer_busy)
                            .label("文件")
                            .on_click(cx.listener(|_, _, _, cx| {
                                Self::choose_attachment(false, cx);
                            })),
                    ),
            )
            .when_some(self.reply_target.clone(), |view, target| {
                view.child(
                    h_flex()
                        .justify_between()
                        .gap_3()
                        .p_3()
                        .rounded_md()
                        .bg(cx.theme().secondary)
                        .child(
                            v_flex()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(format!("回复 {}", target.author)),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(cx.theme().muted_foreground)
                                        .child(truncate_text(&target.body, 96)),
                                ),
                        )
                        .child(
                            Button::new("cancel-reply")
                                .small()
                                .ghost()
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.reply_target = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(self.forward_message_id, |view, message_id| {
                let current_conversation = self
                    .conversations
                    .get(self.selected_conversation)
                    .and_then(|conversation| conversation.id);
                let targets = self
                    .conversations
                    .iter()
                    .enumerate()
                    .filter_map(|(index, conversation)| {
                        let target_id = conversation.id?;
                        (Some(target_id) != current_conversation).then(|| {
                            Button::new(("forward-target", index))
                                .small()
                                .outline()
                                .disabled(self.message_action_busy.is_some())
                                .label(conversation.name.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.forward_message_to(message_id, target_id, cx);
                                }))
                        })
                    })
                    .collect::<Vec<_>>();
                view.child(
                    v_flex()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .bg(cx.theme().secondary)
                        .child(
                            h_flex()
                                .justify_between()
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("转发到会话"),
                                )
                                .child(
                                    Button::new("cancel-forward")
                                        .small()
                                        .ghost()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.forward_message_id = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            v_flex()
                                .max_h(px(160.))
                                .overflow_y_scrollbar()
                                .gap_1()
                                .children(targets),
                        ),
                )
            })
            .when_some(self.pending_upload.clone(), |view, pending| {
                view.child(
                    h_flex()
                        .justify_between()
                        .gap_3()
                        .p_3()
                        .rounded_md()
                        .bg(cx.theme().secondary)
                        .child(div().truncate().child(format!(
                            "{} · {}{}",
                            pending.file_name,
                            format_file_size(pending.byte_size),
                            if pending.image { " · 图片" } else { "" }
                        )))
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("cancel-upload")
                                        .small()
                                        .ghost()
                                        .disabled(self.transfer_busy)
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pending_upload = None;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("send-upload")
                                        .small()
                                        .primary()
                                        .loading(self.transfer_busy)
                                        .label(if self.transfer_busy {
                                            "上传中…"
                                        } else {
                                            "上传并发送"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.upload_pending_attachment(cx);
                                        })),
                                ),
                        ),
                )
            })
            .child(
                h_flex()
                    .items_end()
                    .gap_3()
                    .child(Input::new(&self.composer).h(px(68.)).h_full())
                    .child(
                        Button::new("send")
                            .primary()
                            .large()
                            .label("发送")
                            .on_click(send),
                    ),
            )
            .into_any_element()
    }

    fn render_message(
        &self,
        index: usize,
        message: ChatMessage,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let attachment = message.attachment.clone();
        let client_message_id = message.client_message_id;
        let message_id = message.message_id;
        let action_author = message.author.clone();
        let action_body = message.body.clone();
        let retryable =
            message.outgoing && matches!(message.status.as_str(), "发送失败" | "等待重试");
        let server_actionable = message_id.filter(|_| {
            !matches!(
                message.status.as_str(),
                "发送中" | "发送失败" | "等待重试" | "已撤回"
            )
        });
        let recallable = message.outgoing && server_actionable.is_some();
        let bubble = v_flex()
            .max_w(px(520.))
            .gap_1()
            .p_3()
            .rounded_lg()
            .bg(if message.outgoing {
                cx.theme().primary
            } else {
                cx.theme().secondary
            })
            .text_color(if message.outgoing {
                cx.theme().primary_foreground
            } else {
                cx.theme().secondary_foreground
            })
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(message.author),
            )
            .when_some(message.reply_to, |view, reply_to| {
                view.child(self.render_reply_preview(reply_to, cx))
            })
            .child(div().child(message.body))
            .when_some(attachment, |view, attachment| {
                view.child(
                    h_flex()
                        .justify_between()
                        .gap_2()
                        .child(div().text_size(px(10.)).opacity(0.75).child(format!(
                            "{} · {}",
                            attachment.mime_type,
                            format_file_size(attachment.byte_size)
                        )))
                        .child(
                            Button::new(("download-attachment", index))
                                .small()
                                .outline()
                                .disabled(self.transfer_busy)
                                .label("保存")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    Self::choose_download_destination(attachment.clone(), cx);
                                })),
                        ),
                )
            })
            .when(retryable || server_actionable.is_some(), |view| {
                view.child(self.render_message_actions(
                    MessageActionContext {
                        index,
                        retry_id: client_message_id.filter(|_| retryable),
                        server_id: server_actionable,
                        recallable,
                        author: action_author,
                        body: action_body,
                    },
                    cx,
                ))
            })
            .when_some(
                message_id.filter(|id| self.message_details_open == Some(*id)),
                |view, id| view.child(self.render_message_details(id, cx)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(div().text_size(px(10.)).opacity(0.7).child(message.status))
                    .child(
                        div()
                            .text_size(px(10.))
                            .opacity(0.7)
                            .child(message.timestamp),
                    ),
            );

        let row = h_flex().w_full();
        let row = if message.outgoing {
            row.justify_end()
        } else {
            row.justify_start()
        };
        row.child(bubble).into_any_element()
    }

    fn render_reply_preview(&self, reply_to: MessageId, cx: &Context<Self>) -> AnyElement {
        let label = self
            .messages
            .iter()
            .find(|message| message.message_id == Some(reply_to))
            .map_or_else(
                || "回复一条较早的消息".to_owned(),
                |message| format!("{}：{}", message.author, truncate_text(&message.body, 72)),
            );
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(cx.theme().background.opacity(0.18))
            .text_size(px(11.))
            .opacity(0.82)
            .child(label)
            .into_any_element()
    }

    fn render_message_actions(
        &self,
        action: MessageActionContext,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let MessageActionContext {
            index,
            retry_id,
            server_id,
            recallable,
            author,
            body,
        } = action;
        h_flex()
            .justify_end()
            .gap_1()
            .when_some(retry_id, |actions, id| {
                actions.child(self.render_failed_message_actions(index, id, cx))
            })
            .when_some(server_id, |actions, id| {
                actions.child(
                    self.render_server_message_actions(index, id, recallable, author, body, cx),
                )
            })
            .into_any_element()
    }

    fn render_failed_message_actions(
        &self,
        index: usize,
        message_id: MessageId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_1()
            .child(
                Button::new(("retry-message", index))
                    .small()
                    .outline()
                    .loading(self.message_action_busy == Some(message_id))
                    .disabled(self.outbox_flushing || self.message_action_busy.is_some())
                    .label("立即重试")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.retry_failed_message(message_id, cx);
                    })),
            )
            .child(
                Button::new(("discard-message", index))
                    .small()
                    .danger()
                    .disabled(self.outbox_flushing || self.message_action_busy.is_some())
                    .label(
                        if self.message_confirmation
                            == Some(MessageConfirmation::Discard(message_id))
                        {
                            "确认移除"
                        } else {
                            "移除"
                        },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_or_discard_failed_message(message_id, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_server_message_actions(
        &self,
        index: usize,
        message_id: MessageId,
        recallable: bool,
        author: String,
        body: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .gap_1()
            .child(
                Button::new(("message-details", index))
                    .small()
                    .outline()
                    .selected(self.message_details_open == Some(message_id))
                    .loading(
                        self.message_action_busy == Some(message_id)
                            && !self.message_details.contains_key(&message_id),
                    )
                    .disabled(
                        self.message_action_busy.is_some()
                            && self.message_action_busy != Some(message_id),
                    )
                    .label("详情")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_message_details(message_id, cx);
                    })),
            )
            .child(
                Button::new(("reply-message", index))
                    .small()
                    .outline()
                    .disabled(self.message_action_busy.is_some())
                    .label("回复")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reply_target = Some(ReplyTarget {
                            message_id,
                            author: author.clone(),
                            body: body.clone(),
                        });
                        this.forward_message_id = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("forward-message", index))
                    .small()
                    .outline()
                    .disabled(self.message_action_busy.is_some())
                    .label("转发")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.forward_message_id = Some(message_id);
                        this.reply_target = None;
                        cx.notify();
                    })),
            )
            .when(recallable, |actions| {
                actions.child(
                    Button::new(("recall-message", index))
                        .small()
                        .outline()
                        .loading(self.message_action_busy == Some(message_id))
                        .disabled(self.message_action_busy.is_some())
                        .label(
                            if self.message_confirmation
                                == Some(MessageConfirmation::Recall(message_id))
                            {
                                "确认撤回"
                            } else {
                                "撤回"
                            },
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_or_recall_message(message_id, cx);
                        })),
                )
            })
            .into_any_element()
    }

    fn render_message_details(&self, message_id: MessageId, cx: &mut Context<Self>) -> AnyElement {
        let Some(details) = self.message_details.get(&message_id) else {
            return div()
                .p_2()
                .text_size(px(11.))
                .opacity(0.75)
                .child("正在加载消息详情…")
                .into_any_element();
        };
        let current_user = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.profile.id);
        let reactions = ["👍", "❤️", "😂", "🎉", "👀", "🦀"]
            .into_iter()
            .enumerate()
            .map(|(index, emoji)| {
                let reaction = details
                    .reactions
                    .iter()
                    .find(|reaction| reaction.emoji == emoji);
                let selected = current_user.is_some_and(|user_id| {
                    reaction.is_some_and(|reaction| reaction.user_ids.contains(&user_id))
                });
                Button::new(("message-reaction", index))
                    .small()
                    .outline()
                    .selected(selected)
                    .disabled(self.message_action_busy.is_some())
                    .label(format!(
                        "{emoji} {}",
                        reaction.map_or(0, |reaction| reaction.user_ids.len())
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_message_reaction(message_id, emoji, cx);
                    }))
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap_2()
            .p_2()
            .rounded_md()
            .bg(cx.theme().background.opacity(0.18))
            .child(
                h_flex()
                    .justify_between()
                    .text_size(px(11.))
                    .child(format!("已送达 {}", details.delivered_to.len()))
                    .child(format!("已读 {}", details.read_by.len()))
                    .child(
                        Button::new("favorite-message")
                            .small()
                            .outline()
                            .selected(details.favorited)
                            .disabled(self.message_action_busy.is_some())
                            .label(if details.favorited {
                                "取消收藏"
                            } else {
                                "收藏"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_message_favorite(message_id, cx);
                            })),
                    ),
            )
            .child(h_flex().gap_1().children(reactions))
            .into_any_element()
    }

    fn choose_attachment(image: bool, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(if image {
                "选择图片".into()
            } else {
                "选择文件".into()
            }),
        });
        cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            let _ = this.update(cx, |this, cx| {
                let path = match selected {
                    Ok(Ok(Some(paths))) => paths.into_iter().next(),
                    Ok(Ok(None)) | Err(_) => None,
                    Ok(Err(_)) => {
                        this.message_error = Some("无法打开系统文件选择器".to_owned());
                        cx.notify();
                        return;
                    }
                };
                let Some(path) = path else {
                    return;
                };
                let Ok(metadata) = std::fs::metadata(&path) else {
                    this.message_error = Some("无法读取所选文件".to_owned());
                    cx.notify();
                    return;
                };
                let maximum = if image {
                    25 * 1024 * 1024
                } else {
                    100 * 1024 * 1024
                };
                if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
                    this.message_error = Some(format!(
                        "所选{}大小必须介于 1 B 与 {} MiB 之间",
                        if image { "图片" } else { "文件" },
                        maximum / (1024 * 1024)
                    ));
                    cx.notify();
                    return;
                }
                let Some(file_name) = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
                else {
                    this.message_error = Some("文件名无效".to_owned());
                    cx.notify();
                    return;
                };
                this.pending_upload = Some(PendingUpload {
                    path,
                    file_name,
                    byte_size: metadata.len(),
                    image,
                });
                this.message_error = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_download_destination(attachment: iamrust_domain::Attachment, cx: &mut Context<Self>) {
        let directory = default_download_directory();
        let suggested_name = safe_suggested_file_name(&attachment);
        let receiver = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            let _ = this.update(cx, |this, cx| match selected {
                Ok(Ok(Some(path))) => this.start_attachment_download(attachment, path, cx),
                Ok(Ok(None)) | Err(_) => {}
                Ok(Err(_)) => {
                    this.message_error = Some("无法打开系统保存对话框".to_owned());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_attachment_download(
        &mut self,
        attachment: iamrust_domain::Attachment,
        destination: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.transfer_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.transfer_busy = true;
        self.message_error = Some(format!("正在下载 {}…", attachment.file_name));
        let task = cx
            .background_executor()
            .spawn(async move { api.download_attachment(&attachment, &destination) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.transfer_busy = false;
                this.message_error = Some(match result {
                    Ok(path) => format!("文件已保存到 {}", path.display()),
                    Err(error) => error.user_message(),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn upload_pending_attachment(&mut self, cx: &mut Context<Self>) {
        if self.transfer_busy {
            return;
        }
        let Some(upload) = self.pending_upload.take() else {
            return;
        };
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            self.message_error = Some("请选择真实会话后再发送附件".to_owned());
            self.pending_upload = Some(upload);
            cx.notify();
            return;
        };
        let Some(bootstrap) = self.bootstrap.clone() else {
            self.pending_upload = Some(upload);
            return;
        };
        let Some(api) = self.api.clone() else {
            self.pending_upload = Some(upload);
            return;
        };
        let reply_to = self.reply_target.as_ref().map(|target| target.message_id);
        let client_message_id = MessageId::new();
        self.messages.push(ChatMessage {
            message_id: None,
            client_message_id: Some(client_message_id),
            reply_to,
            author: "我".to_owned(),
            body: format!("[正在上传] {}", upload.file_name),
            outgoing: true,
            timestamp: "现在".to_owned(),
            status: "上传中".to_owned(),
            attachment: None,
        });
        self.transfer_busy = true;
        self.message_error = None;
        self.reply_target = None;
        cx.notify();

        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let sender_id = bootstrap.profile.id;
        let task = cx.background_executor().spawn(async move {
            upload_attachment_and_send(
                api,
                store.as_ref(),
                &runtime,
                AttachmentSendContext {
                    upload,
                    client_message_id,
                    conversation_id,
                    sender_id,
                    reply_to,
                },
            )
        });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_attachment_upload_outcome(outcome);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_attachment_upload_outcome(&mut self, outcome: AttachmentUploadOutcome) {
        self.transfer_busy = false;
        match outcome {
            AttachmentUploadOutcome::Completed {
                label,
                attachment,
                send,
            } => match send {
                SendOutcome::Sent {
                    client_message_id,
                    message_id,
                    cache_warning,
                } => {
                    self.update_message_body(client_message_id, &label);
                    self.update_message_attachment(client_message_id, attachment);
                    self.update_message_sent(client_message_id, message_id);
                    self.cache_error = cache_warning;
                }
                SendOutcome::Failed {
                    client_message_id,
                    error,
                    cache_warning,
                } => {
                    self.update_message_body(client_message_id, &label);
                    self.update_message_attachment(client_message_id, attachment);
                    self.update_message_status(client_message_id, "发送失败");
                    self.message_error = Some(error);
                    self.cache_error = cache_warning;
                }
            },
            AttachmentUploadOutcome::Failed {
                client_message_id,
                error,
            } => {
                self.update_message_status(client_message_id, "上传失败");
                self.message_error = Some(error);
            }
        }
    }

    fn send_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.composer.read(cx).value().trim().to_owned();
        if body.is_empty() {
            return;
        }
        if let Err(error) = iamrust_domain::validate_message_text(&body) {
            self.message_error = Some(error.to_string());
            cx.notify();
            return;
        }
        let reply_to = self.reply_target.as_ref().map(|target| target.message_id);
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            self.messages.push(ChatMessage {
                message_id: None,
                client_message_id: None,
                reply_to,
                author: "我".to_owned(),
                body,
                outgoing: true,
                timestamp: "现在".to_owned(),
                status: "本地演示".to_owned(),
                attachment: None,
            });
            self.composer
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.reply_target = None;
            cx.notify();
            return;
        };
        let Some(bootstrap) = self.bootstrap.clone() else {
            self.message_error = Some("当前会话尚未完成初始化".to_owned());
            cx.notify();
            return;
        };
        let Some(api) = self.api.clone() else {
            self.message_error = Some("服务器地址配置无效".to_owned());
            cx.notify();
            return;
        };
        let client_message_id = MessageId::new();
        let mut pending = Message::pending(
            client_message_id,
            conversation_id,
            bootstrap.profile.id,
            MessageContent::Text { text: body.clone() },
            Utc::now(),
        )
        .expect("validated text must create a pending message");
        pending.reply_to = reply_to;
        self.messages.extend(messages_from_domain(
            std::slice::from_ref(&pending),
            &bootstrap,
        ));
        self.message_error = None;
        self.reply_target = None;
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();

        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let task = cx
            .background_executor()
            .spawn(async move { send_pending_message(api, store.as_ref(), &runtime, pending) });
        cx.spawn(async move |this, cx| {
            let outcome = task.await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    SendOutcome::Sent {
                        client_message_id,
                        message_id,
                        cache_warning,
                    } => {
                        this.update_message_sent(client_message_id, message_id);
                        this.cache_error = cache_warning;
                    }
                    SendOutcome::Failed {
                        client_message_id,
                        error,
                        cache_warning,
                    } => {
                        this.update_message_status(client_message_id, "发送失败");
                        this.message_error = Some(error);
                        this.cache_error = cache_warning;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_message_status(&mut self, client_message_id: MessageId, status: &str) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.client_message_id == Some(client_message_id))
        {
            message.status = status.to_owned();
        }
    }

    fn update_message_sent(&mut self, client_message_id: MessageId, message_id: MessageId) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.client_message_id == Some(client_message_id))
        {
            message.message_id = Some(message_id);
            message.status = "已发送".to_owned();
        }
    }

    fn retry_failed_message(&mut self, client_message_id: MessageId, cx: &mut Context<Self>) {
        if self.outbox_flushing || self.message_action_busy.is_some() {
            return;
        }
        if self.connection_state != ConnectionState::Online {
            self.message_error = Some("当前离线，恢复连接后可立即重试".to_owned());
            cx.notify();
            return;
        }
        let (Some(api), Some(store)) = (self.api.clone(), self.store.clone()) else {
            self.message_error = Some("本地待发送队列不可用".to_owned());
            cx.notify();
            return;
        };
        self.message_action_busy = Some(client_message_id);
        self.message_confirmation = None;
        self.outbox_flushing = true;
        self.update_message_status(client_message_id, "正在重试");
        let runtime = self.runtime.clone();
        let task = cx.background_executor().spawn(async move {
            runtime
                .block_on(store.retry_outbox_now(&client_message_id.to_string()))
                .map(|()| flush_ready_outbox(api, &store, &runtime))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                this.outbox_flushing = false;
                match result {
                    Ok(report) => {
                        let handled = report.sent.iter().any(|(id, _)| *id == client_message_id)
                            || report
                                .retrying
                                .iter()
                                .any(|(id, _)| *id == client_message_id);
                        this.apply_outbox_report(report);
                        if !handled {
                            this.update_message_status(client_message_id, "发送失败");
                            this.message_error = Some("待发送记录不存在，无法重试".to_owned());
                        }
                    }
                    Err(error) => {
                        this.update_message_status(client_message_id, "发送失败");
                        this.message_error = Some(error);
                    }
                }
                this.schedule_outbox_flush(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_or_discard_failed_message(
        &mut self,
        client_message_id: MessageId,
        cx: &mut Context<Self>,
    ) {
        if self.message_confirmation != Some(MessageConfirmation::Discard(client_message_id)) {
            self.message_confirmation = Some(MessageConfirmation::Discard(client_message_id));
            cx.notify();
            return;
        }
        if self.outbox_flushing || self.message_action_busy.is_some() {
            return;
        }
        let Some(store) = self.store.clone() else {
            self.message_error = Some("本地待发送队列不可用".to_owned());
            cx.notify();
            return;
        };
        self.message_action_busy = Some(client_message_id);
        let runtime = self.runtime.clone();
        let task = cx.background_executor().spawn(async move {
            runtime.block_on(store.discard_pending_message(&client_message_id.to_string()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                this.message_confirmation = None;
                match result {
                    Ok(()) => {
                        this.messages
                            .retain(|message| message.client_message_id != Some(client_message_id));
                        this.message_error = Some("待发送消息已从本机移除".to_owned());
                    }
                    Err(error) => this.message_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_or_recall_message(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        if self.message_confirmation != Some(MessageConfirmation::Recall(message_id)) {
            self.message_confirmation = Some(MessageConfirmation::Recall(message_id));
            cx.notify();
            return;
        }
        if self.message_action_busy.is_some() {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.message_action_busy = Some(message_id);
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let task = cx.background_executor().spawn(async move {
            api.recall_message(message_id).map(|message| {
                let cache_warning = store.as_ref().and_then(|store| {
                    runtime
                        .block_on(store.cache_messages(std::slice::from_ref(&message)))
                        .err()
                });
                (message, cache_warning)
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                this.message_confirmation = None;
                match result {
                    Ok((message, cache_warning)) => {
                        this.apply_realtime_message(&message);
                        this.cache_error = cache_warning;
                        this.message_error = Some("消息已撤回".to_owned());
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn forward_message_to(
        &mut self,
        message_id: MessageId,
        target_conversation_id: ConversationId,
        cx: &mut Context<Self>,
    ) {
        if self.message_action_busy.is_some() || self.forward_message_id != Some(message_id) {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.message_action_busy = Some(message_id);
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let task = cx.background_executor().spawn(async move {
            api.forward_message(message_id, target_conversation_id)
                .map(|messages| {
                    let cache_warning = store
                        .as_ref()
                        .and_then(|store| runtime.block_on(store.cache_messages(&messages)).err());
                    (messages, cache_warning)
                })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                match result {
                    Ok((messages, cache_warning)) => {
                        this.forward_message_id = None;
                        for message in messages {
                            this.apply_realtime_message(&message);
                        }
                        this.cache_error = cache_warning;
                        this.message_error = Some("消息已转发".to_owned());
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_message_details(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        if self.message_details_open == Some(message_id) {
            self.message_details_open = None;
            cx.notify();
            return;
        }
        self.message_details_open = Some(message_id);
        if self.message_details.contains_key(&message_id) {
            cx.notify();
            return;
        }
        if self.message_action_busy.is_some() {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.message_action_busy = Some(message_id);
        let task = cx
            .background_executor()
            .spawn(async move { api.message_details(message_id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                match result {
                    Ok(details) => {
                        this.message_details.insert(message_id, details);
                    }
                    Err(error) => {
                        this.message_details_open = None;
                        this.message_error = Some(error.user_message());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_message_reaction(
        &mut self,
        message_id: MessageId,
        emoji: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.message_action_busy.is_some() {
            return;
        }
        let Some(user_id) = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.profile.id)
        else {
            return;
        };
        let Some(details) = self.message_details.get(&message_id) else {
            return;
        };
        let active = !details
            .reactions
            .iter()
            .find(|reaction| reaction.emoji == emoji)
            .is_some_and(|reaction| reaction.user_ids.contains(&user_id));
        let Some(api) = self.api.clone() else {
            return;
        };
        self.message_action_busy = Some(message_id);
        let task = cx
            .background_executor()
            .spawn(async move { api.react_to_message(message_id, emoji, active) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                match result {
                    Ok(reactions) => {
                        if let Some(details) = this.message_details.get_mut(&message_id) {
                            details.reactions = reactions;
                        }
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_message_favorite(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        if self.message_action_busy.is_some() {
            return;
        }
        let Some(favorite) = self
            .message_details
            .get(&message_id)
            .map(|details| !details.favorited)
        else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.message_action_busy = Some(message_id);
        let task = cx
            .background_executor()
            .spawn(async move { api.favorite_message(message_id, favorite) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.message_action_busy = None;
                match result {
                    Ok(()) => {
                        if let Some(details) = this.message_details.get_mut(&message_id) {
                            details.favorited = favorite;
                        }
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn update_message_body(&mut self, client_message_id: MessageId, body: &str) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.client_message_id == Some(client_message_id))
        {
            message.body = body.to_owned();
        }
    }

    fn update_message_attachment(
        &mut self,
        client_message_id: MessageId,
        attachment: iamrust_domain::Attachment,
    ) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.client_message_id == Some(client_message_id))
        {
            message.attachment = Some(attachment);
        }
    }

    fn select_conversation(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.conversations.len() {
            return;
        }
        self.persist_current_draft(cx);
        self.selected_conversation = index;
        self.group_details_open = false;
        self.reset_group_details();
        self.message_error = None;
        self.timeline_next_cursor = None;
        self.timeline_loading_older = false;
        self.message_action_busy = None;
        self.message_confirmation = None;
        self.reply_target = None;
        self.forward_message_id = None;
        self.message_details.clear();
        self.message_details_open = None;
        let draft = self
            .conversations
            .get(index)
            .and_then(|conversation| conversation.id)
            .and_then(|conversation_id| {
                self.bootstrap.as_ref().and_then(|bootstrap| {
                    bootstrap
                        .conversation_states
                        .iter()
                        .find(|state| state.conversation_id == conversation_id)
                        .map(|state| state.draft.clone())
                })
            })
            .unwrap_or_default();
        self.composer
            .update(cx, |state, cx| state.set_value(draft, window, cx));
        self.load_selected_timeline(cx);
        cx.notify();
    }

    fn selected_conversation_data(&self) -> Option<&Conversation> {
        let conversation_id = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)?;
        self.bootstrap
            .as_ref()?
            .conversations
            .iter()
            .find(|conversation| conversation.id == conversation_id)
    }

    fn reset_group_details(&mut self) {
        self.group_details_loading = false;
        self.group_mute_all = false;
        self.group_announcements.clear();
        self.group_files.clear();
        self.group_join_requests.clear();
        self.group_polls.clear();
        self.group_invite_members.clear();
        self.selected_poll_options.clear();
        self.group_confirmation = None;
    }

    fn toggle_conversation_details(&mut self, cx: &mut Context<Self>) {
        self.group_details_open = !self.group_details_open;
        self.reset_group_details();
        if self.group_details_open
            && self
                .selected_conversation_data()
                .is_some_and(|conversation| {
                    matches!(conversation.kind, ConversationKind::Group { .. })
                })
        {
            self.load_group_details(cx);
        }
        cx.notify();
    }

    fn load_group_details(&mut self, cx: &mut Context<Self>) {
        if self.group_details_loading {
            return;
        }
        let Some(conversation_id) = self.selected_conversation_data().and_then(|conversation| {
            matches!(conversation.kind, ConversationKind::Group { .. }).then_some(conversation.id)
        }) else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.group_details_loading = true;
        let task = cx.background_executor().spawn(async move {
            let settings = api.group_settings(conversation_id)?;
            let announcements = api.group_announcements(conversation_id)?;
            let files = api.group_files(conversation_id)?;
            let polls = api.group_polls(conversation_id)?;
            let join_requests = api.group_join_requests(conversation_id).unwrap_or_default();
            Ok::<_, ClientError>(GroupDetailsData {
                mute_all: settings.mute_all,
                announcements,
                files,
                join_requests,
                polls,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .selected_conversation_data()
                    .map(|conversation| conversation.id)
                    != Some(conversation_id)
                {
                    return;
                }
                this.group_details_loading = false;
                match result {
                    Ok(details) => {
                        this.group_mute_all = details.mute_all;
                        this.group_announcements = details.announcements;
                        this.group_files = details.files;
                        this.group_join_requests = details.join_requests;
                        this.group_polls = details.polls;
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn schedule_draft_save(&mut self, cx: &mut Context<Self>) {
        self.draft_revision = self.draft_revision.wrapping_add(1);
        let revision = self.draft_revision;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(500)).await;
            let _ = this.update(cx, |this, cx| {
                if this.draft_revision == revision {
                    this.persist_current_draft(cx);
                }
            });
        })
        .detach();
    }

    fn schedule_typing_stop(&mut self, cx: &mut Context<Self>) {
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            return;
        };
        let Some(realtime) = self.realtime.clone() else {
            return;
        };
        realtime.send_typing(conversation_id, true);
        self.typing_revision = self.typing_revision.wrapping_add(1);
        let revision = self.typing_revision;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_secs(3)).await;
            let _ = this.update(cx, |this, _| {
                if this.typing_revision == revision {
                    realtime.send_typing(conversation_id, false);
                }
            });
        })
        .detach();
    }

    fn start_call(&mut self, video: bool, cx: &mut Context<Self>) {
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            self.message_error = Some("请选择真实会话后再发起通话".to_owned());
            cx.notify();
            return;
        };
        let Some(realtime) = &self.realtime else {
            self.message_error = Some("实时连接不可用，暂时无法发起通话".to_owned());
            cx.notify();
            return;
        };
        realtime.send_call(
            conversation_id,
            Uuid::new_v4(),
            None,
            CallSignal::Invite { video },
        );
        self.message_error = Some(if video {
            "已发送视频通话邀请".to_owned()
        } else {
            "已发送语音通话邀请".to_owned()
        });
        cx.notify();
    }

    fn chat_presence_label(&self) -> String {
        let typing = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
            .and_then(|conversation_id| self.typing_users.get(&conversation_id))
            .map(HashSet::len)
            .unwrap_or_default();
        if typing > 0 {
            return if typing == 1 {
                "对方正在输入…".to_owned()
            } else {
                format!("{typing} 人正在输入…")
            };
        }
        if self.offline {
            "离线 · 显示本地缓存".to_owned()
        } else {
            self.connection_state.label().to_owned()
        }
    }

    fn persist_current_draft(&mut self, cx: &mut Context<Self>) {
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            return;
        };
        let draft = self.composer.read(cx).value().to_string();
        if let Some(state) = self.bootstrap.as_mut().and_then(|bootstrap| {
            bootstrap
                .conversation_states
                .iter_mut()
                .find(|state| state.conversation_id == conversation_id)
        }) {
            state.draft.clone_from(&draft);
        }
        if let Some(preview) = self.conversations.get_mut(self.selected_conversation)
            && !draft.trim().is_empty()
        {
            preview.summary = format!("草稿：{}", draft.trim());
        }
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let api = self.api.clone();
        cx.background_executor()
            .spawn(async move {
                if let Some(store) = store {
                    let _ =
                        runtime.block_on(store.save_draft(&conversation_id.to_string(), &draft));
                }
                if let Some(api) = api {
                    let _ = api.save_draft(conversation_id, draft);
                }
            })
            .detach();
    }

    fn load_selected_timeline(&mut self, cx: &mut Context<Self>) {
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            return;
        };
        let api = self.api.clone();
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        self.timeline_generation = self.timeline_generation.wrapping_add(1);
        let generation = self.timeline_generation;
        self.messages_loading = true;
        self.timeline_loading_older = false;
        self.timeline_next_cursor = None;
        self.message_confirmation = None;
        self.message_error = None;
        let task = cx.background_executor().spawn(async move {
            load_timeline(api.as_deref(), store.as_ref(), &runtime, conversation_id)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let still_selected = this
                    .conversations
                    .get(this.selected_conversation)
                    .and_then(|conversation| conversation.id)
                    == Some(conversation_id);
                if generation != this.timeline_generation || !still_selected {
                    return;
                }
                this.messages_loading = false;
                match result {
                    TimelineOutcome::Live(messages, next_cursor, cache_warning) => {
                        this.offline = false;
                        this.cache_error = cache_warning;
                        this.apply_timeline(messages, next_cursor, cx);
                    }
                    TimelineOutcome::Cached(messages, error) => {
                        this.offline = true;
                        this.message_error = Some(format!("{error}；已显示本地消息"));
                        this.apply_timeline(messages, None, cx);
                    }
                    TimelineOutcome::Failed(error) => this.message_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_older_messages(&mut self, cx: &mut Context<Self>) {
        if self.messages_loading || self.timeline_loading_older {
            return;
        }
        let Some(before) = self.timeline_next_cursor else {
            return;
        };
        let Some(conversation_id) = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id)
        else {
            return;
        };
        let Some(api) = self.api.clone() else {
            self.timeline_next_cursor = None;
            return;
        };
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        let generation = self.timeline_generation;
        self.timeline_loading_older = true;
        self.message_error = None;
        let task = cx.background_executor().spawn(async move {
            let page = api
                .messages(conversation_id, Some(before), 50)
                .map_err(|error| error.user_message())?;
            let next_cursor =
                page_next_cursor(&page.items, page.next_cursor.as_deref(), Some(before));
            let cache_warning = store
                .as_ref()
                .and_then(|store| runtime.block_on(store.cache_messages(&page.items)).err());
            Ok::<_, String>((page.items, next_cursor, cache_warning))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let still_selected = this
                    .conversations
                    .get(this.selected_conversation)
                    .and_then(|conversation| conversation.id)
                    == Some(conversation_id);
                if generation != this.timeline_generation || !still_selected {
                    return;
                }
                this.timeline_loading_older = false;
                match result {
                    Ok((messages, next_cursor, cache_warning)) => {
                        this.cache_error = cache_warning;
                        this.prepend_timeline(messages, next_cursor);
                    }
                    Err(error) => this.message_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_timeline(
        &mut self,
        mut messages: Vec<Message>,
        next_cursor: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        messages.sort_by(|left, right| {
            left.sequence
                .unwrap_or(u64::MAX)
                .cmp(&right.sequence.unwrap_or(u64::MAX))
                .then_with(|| left.created_at.cmp(&right.created_at))
        });
        if let Some(bootstrap) = &self.bootstrap {
            self.messages = messages_from_domain(&messages, bootstrap);
        }
        self.timeline_next_cursor = next_cursor;
        if let Some(preview) = self.conversations.get_mut(self.selected_conversation) {
            preview.unread = 0;
            if let Some(last) = self.messages.last() {
                preview.summary.clone_from(&last.body);
                preview.timestamp.clone_from(&last.timestamp);
            }
        }
        let last_sequence = messages.iter().filter_map(|message| message.sequence).max();
        let conversation_id = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id);
        if let (Some(api), Some(conversation_id), Some(last_sequence)) =
            (self.api.clone(), conversation_id, last_sequence)
        {
            cx.background_executor()
                .spawn(async move {
                    let _ = api.mark_read(conversation_id, last_sequence);
                })
                .detach();
        }
    }

    fn prepend_timeline(&mut self, mut messages: Vec<Message>, next_cursor: Option<u64>) {
        messages.sort_by(|left, right| {
            left.sequence
                .unwrap_or(u64::MAX)
                .cmp(&right.sequence.unwrap_or(u64::MAX))
                .then_with(|| left.created_at.cmp(&right.created_at))
        });
        let mut existing = self
            .messages
            .iter()
            .filter_map(|message| message.client_message_id)
            .collect::<HashSet<_>>();
        messages.retain(|message| existing.insert(message.client_message_id));
        if let Some(bootstrap) = &self.bootstrap {
            let mut rendered = messages_from_domain(&messages, bootstrap);
            rendered.append(&mut self.messages);
            self.messages = rendered;
        }
        self.timeline_next_cursor = next_cursor;
    }

    fn start_authentication(&mut self, cx: &mut Context<Self>) {
        if self.auth_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            self.auth_error = Some("服务器地址配置无效".to_owned());
            cx.notify();
            return;
        };
        let mode = self.auth_mode;
        let login = self.account.read(cx).value().trim().to_owned();
        let email = self.register_email.read(cx).value().trim().to_owned();
        let username = self.register_username.read(cx).value().trim().to_owned();
        let nickname = self.register_nickname.read(cx).value().trim().to_owned();
        let password = self.password.read(cx).value().to_string();
        let second_factor_code = self.second_factor_code.read(cx).value().trim().to_owned();
        let reset_email = self.reset_email.read(cx).value().trim().to_owned();
        let reset_token = self.reset_token.read(cx).value().trim().to_owned();
        let new_password = self.new_password.read(cx).value().to_string();
        let missing_message = match mode {
            AuthMode::Login if login.is_empty() || password.is_empty() => Some("请输入账号和密码"),
            AuthMode::Register
                if email.is_empty()
                    || username.is_empty()
                    || nickname.is_empty()
                    || password.is_empty() =>
            {
                Some("请填写邮箱、用户名、昵称和密码")
            }
            AuthMode::PasswordReset if reset_email.is_empty() => Some("请输入注册邮箱"),
            AuthMode::PasswordResetConfirm if reset_token.is_empty() || new_password.is_empty() => {
                Some("请输入重置令牌和新密码")
            }
            _ => None,
        };
        if let Some(message) = missing_message {
            self.auth_error = Some(message.to_owned());
            self.auth_notice = None;
            cx.notify();
            return;
        }
        self.auth_busy = true;
        self.auth_error = None;
        self.auth_notice = None;
        cx.notify();

        let task = cx.background_executor().spawn(async move {
            match mode {
                AuthMode::Login => {
                    api.login(&login, &password, Some(&second_factor_code))?;
                    Ok::<AuthOutcome, ClientError>(AuthOutcome::Authenticated(Box::new(
                        api.bootstrap()?,
                    )))
                }
                AuthMode::Register => {
                    api.register(&email, &username, &password, &nickname)?;
                    Ok(AuthOutcome::Authenticated(Box::new(api.bootstrap()?)))
                }
                AuthMode::PasswordReset => {
                    api.request_password_reset(&reset_email)?;
                    Ok(AuthOutcome::ResetRequested)
                }
                AuthMode::PasswordResetConfirm => {
                    api.confirm_password_reset(&reset_token, &new_password)?;
                    Ok(AuthOutcome::ResetConfirmed)
                }
                AuthMode::QrLogin => Ok(AuthOutcome::QrStarted(api.begin_qr_login()?)),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.auth_busy = false;
                match result {
                    Ok(AuthOutcome::Authenticated(bootstrap)) => {
                        this.apply_bootstrap(*bootstrap, true, cx);
                    }
                    Ok(AuthOutcome::ResetRequested) => {
                        this.auth_mode = AuthMode::PasswordResetConfirm;
                        this.auth_notice = Some(
                            "如果邮箱已注册，重置令牌已发送；请在下方输入令牌和新密码。".to_owned(),
                        );
                    }
                    Ok(AuthOutcome::ResetConfirmed) => {
                        this.auth_mode = AuthMode::Login;
                        this.auth_notice = Some("密码已重置，请使用新密码登录。".to_owned());
                    }
                    Ok(AuthOutcome::QrStarted(challenge)) => {
                        this.qr_challenge = Some(challenge);
                        this.qr_generation = this.qr_generation.wrapping_add(1);
                        this.auth_notice = Some("等待已登录设备确认…".to_owned());
                        this.schedule_qr_poll(cx);
                    }
                    Err(error) => this.auth_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn schedule_qr_poll(&mut self, cx: &mut Context<Self>) {
        let generation = self.qr_generation;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this.qr_generation == generation && this.auth_mode == AuthMode::QrLogin {
                    this.poll_qr_login(generation, cx);
                }
            });
        })
        .detach();
    }

    fn poll_qr_login(&mut self, generation: u64, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let Some(challenge) = self.qr_challenge.clone() else {
            return;
        };
        if Utc::now() >= challenge.expires_at {
            self.qr_generation = self.qr_generation.wrapping_add(1);
            self.qr_challenge = None;
            self.auth_notice = None;
            self.auth_error = Some("登录二维码已过期，请重新生成。".to_owned());
            cx.notify();
            return;
        }
        let task = cx.background_executor().spawn(async move {
            match api.poll_qr_login(&challenge)? {
                Some(_) => Ok::<Option<BootstrapResponse>, ClientError>(Some(api.bootstrap()?)),
                None => Ok(None),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.qr_generation != generation || this.auth_mode != AuthMode::QrLogin {
                    return;
                }
                match result {
                    Ok(Some(bootstrap)) => {
                        this.qr_generation = this.qr_generation.wrapping_add(1);
                        this.qr_challenge = None;
                        this.auth_notice = None;
                        this.apply_bootstrap(bootstrap, true, cx);
                    }
                    Ok(None) => this.schedule_qr_poll(cx),
                    Err(error) => {
                        this.auth_notice = None;
                        this.auth_error = Some(error.user_message());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn restore_session(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.auth_busy = true;
        let runtime = self.runtime.clone();
        let store = self.store.clone();
        let task = cx.background_executor().spawn(async move {
            match api.restore() {
                Ok(Some(_)) => match api.bootstrap() {
                    Ok(bootstrap) => RestoreOutcome::Live(bootstrap),
                    Err(error) => cached_or_failed(&runtime, store.as_ref(), error.user_message()),
                },
                Ok(None) => RestoreOutcome::LoggedOut,
                Err(error) => cached_or_failed(&runtime, store.as_ref(), error.user_message()),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.auth_busy = false;
                match result {
                    RestoreOutcome::Live(bootstrap) => {
                        this.apply_bootstrap(bootstrap, true, cx);
                    }
                    RestoreOutcome::Cached(bootstrap, reason) => {
                        this.apply_bootstrap(bootstrap, false, cx);
                        this.cache_error = Some(reason);
                    }
                    RestoreOutcome::LoggedOut => {}
                    RestoreOutcome::Failed(error) => this.auth_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_realtime(&mut self, cursor: u64, cx: &mut Context<Self>) {
        if let Some(realtime) = self.realtime.take() {
            realtime.stop();
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.realtime_generation = self.realtime_generation.wrapping_add(1);
        let generation = self.realtime_generation;
        let (handle, mut events) = realtime::spawn(&self.runtime, api, cursor);
        self.realtime = Some(handle);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let result = this.update(cx, |this, cx| {
                    if this.realtime_generation == generation {
                        this.handle_realtime_event(event, cx);
                        cx.notify();
                    }
                });
                if result.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_realtime_event(&mut self, event: RealtimeEvent, cx: &mut Context<Self>) {
        match event {
            RealtimeEvent::State(state) => {
                if self.connection_state != state {
                    self.outbox_flush_generation = self.outbox_flush_generation.wrapping_add(1);
                }
                self.connection_state = state;
                self.offline = matches!(state, ConnectionState::Offline | ConnectionState::Failed);
                if state == ConnectionState::Online {
                    self.flush_outbox(cx);
                }
            }
            RealtimeEvent::Sync(event) => self.handle_sync_event(event, cx),
            RealtimeEvent::Typing {
                conversation_id,
                user_id,
                active,
            } => {
                let users = self.typing_users.entry(conversation_id).or_default();
                if active {
                    users.insert(user_id);
                } else {
                    users.remove(&user_id);
                }
                if users.is_empty() {
                    self.typing_users.remove(&conversation_id);
                }
            }
            RealtimeEvent::Call {
                conversation_id,
                call_id,
                from_user_id,
                signal,
            } => match signal {
                CallSignal::Invite { video } => {
                    self.incoming_call = Some((conversation_id, call_id, video));
                    self.message_error = Some(if video {
                        format!("收到来自 {from_user_id} 的视频通话邀请")
                    } else {
                        format!("收到来自 {from_user_id} 的语音通话邀请")
                    });
                }
                CallSignal::Hangup | CallSignal::Reject => {
                    self.incoming_call = None;
                    self.message_error = Some("通话已结束".to_owned());
                }
                _ => {}
            },
            RealtimeEvent::Error(error) => self.message_error = Some(error),
        }
    }

    fn flush_outbox(&mut self, cx: &mut Context<Self>) {
        if self.outbox_flushing || self.connection_state != ConnectionState::Online {
            return;
        }
        let (Some(api), Some(store)) = (self.api.clone(), self.store.clone()) else {
            return;
        };
        self.outbox_flushing = true;
        let runtime = self.runtime.clone();
        let task = cx
            .background_executor()
            .spawn(async move { flush_ready_outbox(api, &store, &runtime) });
        cx.spawn(async move |this, cx| {
            let report = task.await;
            let _ = this.update(cx, |this, cx| {
                this.outbox_flushing = false;
                this.apply_outbox_report(report);
                this.schedule_outbox_flush(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_outbox_report(&mut self, report: OutboxFlushReport) {
        for (client_message_id, message_id) in report.sent {
            self.update_message_sent(client_message_id, message_id);
        }
        for (message_id, error) in report.retrying {
            self.update_message_status(message_id, "等待重试");
            self.message_error = Some(error);
        }
        if report.cache_warning.is_some() {
            self.cache_error = report.cache_warning;
        }
    }

    fn schedule_outbox_flush(&mut self, cx: &mut Context<Self>) {
        let generation = self.outbox_flush_generation;
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_secs(15)).await;
            let _ = this.update(cx, |this, cx| {
                if this.connection_state == ConnectionState::Online
                    && this.outbox_flush_generation == generation
                {
                    this.flush_outbox(cx);
                }
            });
        })
        .detach();
    }

    fn handle_sync_event(&mut self, event: iamrust_domain::SyncEvent, cx: &mut Context<Self>) {
        let is_new_message = matches!(event.kind, EventKind::MessageCreated);
        let message = matches!(
            event.kind,
            EventKind::MessageCreated | EventKind::MessageUpdated
        )
        .then(|| event.payload.get("message").cloned())
        .flatten()
        .and_then(|value| serde_json::from_value::<Message>(value).ok());

        if let Some(message) = message.clone() {
            if is_new_message {
                self.maybe_notify_for_message(&message, cx);
            }
            self.apply_realtime_message(&message);
        } else {
            self.refresh_bootstrap(cx);
        }
        if let Some(store) = self.store.clone() {
            let runtime = self.runtime.clone();
            cx.background_executor()
                .spawn(async move {
                    let _ = runtime.block_on(store.record_sync_event(&event));
                    if let Some(message) = message {
                        let _ = runtime.block_on(store.cache_messages(&[message]));
                    }
                })
                .detach();
        }
    }

    fn maybe_notify_for_message(&self, message: &Message, cx: &mut Context<Self>) {
        if self.window_active || !self.authenticated || !self.notifications_enabled {
            return;
        }
        let Some(bootstrap) = &self.bootstrap else {
            return;
        };
        if message.sender_id == bootstrap.profile.id {
            return;
        }
        let timestamp = message.server_created_at.unwrap_or(message.created_at);
        let age_seconds = Utc::now().signed_duration_since(timestamp).num_seconds();
        if !(-10..=45).contains(&age_seconds) {
            return;
        }
        let Some(conversation) = self
            .conversations
            .iter()
            .find(|conversation| conversation.id == Some(message.conversation_id))
        else {
            return;
        };
        if conversation.muted {
            return;
        }

        let rendered = messages_from_domain(std::slice::from_ref(message), bootstrap)
            .into_iter()
            .next()
            .expect("one message must render once");
        let (title, body) = notification_text(
            &conversation.name,
            &rendered.author,
            &rendered.body,
            self.privacy_mode,
        );
        let play_sound = self.sounds_enabled;
        cx.background_executor()
            .spawn(async move {
                desktop::show_message_notification(title, body, play_sound);
            })
            .detach();
    }

    fn apply_realtime_message(&mut self, message: &Message) {
        self.message_details.remove(&message.id);
        if self.message_details_open == Some(message.id)
            && message.status == MessageStatus::Recalled
        {
            self.message_details_open = None;
        }
        let Some(bootstrap) = &self.bootstrap else {
            return;
        };
        let Some(preview_index) = self
            .conversations
            .iter()
            .position(|preview| preview.id == Some(message.conversation_id))
        else {
            return;
        };
        let rendered = messages_from_domain(std::slice::from_ref(message), bootstrap)
            .into_iter()
            .next()
            .expect("one message must render once");
        if preview_index == self.selected_conversation {
            if let Some(existing) = self
                .messages
                .iter_mut()
                .find(|existing| existing.client_message_id == rendered.client_message_id)
            {
                *existing = rendered.clone();
            } else {
                self.messages.push(rendered.clone());
            }
        } else if message.sender_id != bootstrap.profile.id {
            self.conversations[preview_index].unread =
                self.conversations[preview_index].unread.saturating_add(1);
        }
        self.conversations[preview_index]
            .summary
            .clone_from(&rendered.body);
        self.conversations[preview_index]
            .timestamp
            .clone_from(&rendered.timestamp);
    }

    fn refresh_bootstrap(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let selected = self
            .conversations
            .get(self.selected_conversation)
            .and_then(|conversation| conversation.id);
        let task = cx
            .background_executor()
            .spawn(async move { api.bootstrap() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(bootstrap) => {
                    this.profile_name = if bootstrap.profile.nickname.trim().is_empty() {
                        bootstrap.profile.username.clone()
                    } else {
                        bootstrap.profile.nickname.clone()
                    };
                    this.conversations = conversations_from_bootstrap(&bootstrap);
                    this.selected_conversation = selected
                        .and_then(|id| {
                            this.conversations
                                .iter()
                                .position(|conversation| conversation.id == Some(id))
                        })
                        .unwrap_or_default();
                    this.bootstrap = Some(bootstrap.clone());
                    this.persist_bootstrap(bootstrap, cx);
                    cx.notify();
                }
                Err(error) => {
                    this.message_error = Some(error.user_message());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_logout(&mut self, cx: &mut Context<Self>) {
        if let Some(realtime) = self.realtime.take() {
            realtime.stop();
        }
        self.realtime_generation = self.realtime_generation.wrapping_add(1);
        self.authenticated = false;
        self.navigation = Navigation::Chats;
        self.auth_error = None;
        self.auth_notice = None;
        self.qr_generation = self.qr_generation.wrapping_add(1);
        self.qr_challenge = None;
        self.profile_name = "我".to_owned();
        self.conversations = demo_conversations();
        self.messages = demo_messages();
        self.offline = false;
        self.bootstrap = None;
        self.messages_loading = false;
        self.timeline_loading_older = false;
        self.timeline_next_cursor = None;
        self.timeline_generation = self.timeline_generation.wrapping_add(1);
        self.message_error = None;
        self.message_action_busy = None;
        self.message_confirmation = None;
        self.reply_target = None;
        self.forward_message_id = None;
        self.message_details.clear();
        self.message_details_open = None;
        self.connection_state = ConnectionState::Offline;
        self.typing_users.clear();
        self.incoming_call = None;
        self.outbox_flushing = false;
        self.outbox_flush_generation = self.outbox_flush_generation.wrapping_add(1);
        self.local_search_results.clear();
        self.local_search_busy = false;
        self.local_search_error = None;
        self.group_details_open = false;
        self.reset_group_details();
        self.devices.clear();
        self.security_loading = false;
        self.security_loaded = false;
        self.second_factor_status = None;
        self.second_factor_setup = None;
        self.recovery_codes.clear();
        self.revoke_device_confirmation = None;
        if !self.retain_cache_on_logout
            && let Some(store) = self.store.clone()
        {
            let runtime = self.runtime.clone();
            cx.background_executor()
                .spawn(async move {
                    let _ = runtime.block_on(store.clear_account_cache());
                })
                .detach();
        }
        if let Some(api) = self.api.clone() {
            cx.background_executor()
                .spawn(async move {
                    api.logout();
                })
                .detach();
        }
        cx.notify();
    }

    fn apply_bootstrap(
        &mut self,
        bootstrap: iamrust_protocol::BootstrapResponse,
        online: bool,
        cx: &mut Context<Self>,
    ) {
        self.profile_name = if bootstrap.profile.nickname.trim().is_empty() {
            bootstrap.profile.username.clone()
        } else {
            bootstrap.profile.nickname.clone()
        };
        self.conversations = conversations_from_bootstrap(&bootstrap);
        self.bootstrap = Some(bootstrap.clone());
        self.messages.clear();
        self.timeline_loading_older = false;
        self.timeline_next_cursor = None;
        self.timeline_generation = self.timeline_generation.wrapping_add(1);
        self.message_action_busy = None;
        self.message_confirmation = None;
        self.reply_target = None;
        self.forward_message_id = None;
        self.message_details.clear();
        self.message_details_open = None;
        self.selected_conversation = 0;
        self.auth_error = None;
        self.auth_notice = None;
        self.qr_generation = self.qr_generation.wrapping_add(1);
        self.qr_challenge = None;
        self.authenticated = true;
        self.outbox_flushing = false;
        self.outbox_flush_generation = self.outbox_flush_generation.wrapping_add(1);
        self.local_search_results.clear();
        self.local_search_busy = false;
        self.local_search_error = None;
        self.group_details_open = false;
        self.reset_group_details();
        self.security_loading = false;
        self.security_loaded = false;
        self.second_factor_status = None;
        self.second_factor_setup = None;
        self.recovery_codes.clear();
        self.revoke_device_confirmation = None;
        self.offline = !online;
        self.connection_state = if online {
            ConnectionState::Connecting
        } else {
            ConnectionState::Offline
        };
        self.start_realtime(bootstrap.cursor, cx);
        if online {
            self.persist_bootstrap(bootstrap, cx);
        }
        self.load_selected_timeline(cx);
    }

    fn persist_bootstrap(&self, bootstrap: BootstrapResponse, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        let task = cx
            .background_executor()
            .spawn(async move { runtime.block_on(store.cache_bootstrap(&bootstrap)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(error) = result {
                let _ = this.update(cx, |this, cx| {
                    this.cache_error = Some(error);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn filtered_friends(&self, cx: &Context<Self>) -> Vec<iamrust_domain::UserProfile> {
        let query = self.contact_filter.read(cx).value().trim().to_lowercase();
        self.bootstrap
            .as_ref()
            .map(|bootstrap| {
                bootstrap
                    .friends
                    .iter()
                    .filter(|profile| {
                        query.is_empty()
                            || profile.username.to_lowercase().contains(&query)
                            || profile.nickname.to_lowercase().contains(&query)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn render_group_builder(
        &self,
        friends: &[iamrust_domain::UserProfile],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let members = friends.iter().cloned().enumerate().map(|(index, profile)| {
            let user_id = profile.id;
            Button::new(("group-member", index))
                .small()
                .selected(self.selected_group_members.contains(&user_id))
                .label(profile_name(&profile))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.selected_group_members.insert(user_id) {
                        this.selected_group_members.remove(&user_id);
                    }
                    cx.notify();
                }))
        });
        v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(Input::new(&self.group_name).small().cleanable(true))
            .child(h_flex().flex_wrap().gap_1().children(members))
            .child(
                Button::new("create-group")
                    .primary()
                    .small()
                    .disabled(self.selected_group_members.is_empty() || self.action_busy)
                    .label(format!(
                        "创建群聊（{} 人）",
                        self.selected_group_members.len() + 1
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.start_create_group(cx))),
            )
            .into_any_element()
    }

    fn start_create_group(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let name = self.group_name.read(cx).value().trim().to_owned();
        if name.is_empty() || self.selected_group_members.is_empty() {
            self.action_error = Some("请输入群名称并选择至少一位好友".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        let member_ids = self.selected_group_members.iter().copied().collect();
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.create_group(name, member_ids) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(_) => {
                        this.selected_group_members.clear();
                        this.creating_group = false;
                        this.navigation = Navigation::Chats;
                        this.refresh_bootstrap(cx);
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_contact_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let friends = self.filtered_friends(cx);
        let group_friends = friends.clone();
        let rows = friends
            .into_iter()
            .enumerate()
            .map(|(index, profile)| self.render_contact_row(index, profile, cx))
            .collect::<Vec<_>>();
        let pending = self.pending_friend_requests(cx);

        v_flex()
            .h_full()
            .w(px(320.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("联系人"),
                            )
                            .child(
                                Button::new("toggle-create-group")
                                    .small()
                                    .outline()
                                    .selected(self.creating_group)
                                    .label("建群")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.creating_group = !this.creating_group;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(Input::new(&self.contact_filter).small().cleanable(true))
                    .when(self.creating_group, |header| {
                        header.child(self.render_group_builder(&group_friends, cx))
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .p_2()
                    .gap_1()
                    .when(!pending.is_empty(), |list| {
                        list.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("待处理申请 · {}", pending.len())),
                        )
                    })
                    .children(pending)
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child("好友"),
                    )
                    .children(rows),
            )
            .into_any_element()
    }

    fn render_contact_row(
        &self,
        index: usize,
        profile: iamrust_domain::UserProfile,
        cx: &mut Context<Self>,
    ) -> Button {
        let name = profile_name(&profile);
        Button::new(("contact", index))
            .ghost()
            .selected(index == self.selected_contact)
            .w_full()
            .h(px(62.))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_contact = index;
                this.delete_friend_confirmation = None;
                cx.notify();
            }))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .child(Avatar::new().name(name.clone()))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(name))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(profile.signature),
                            ),
                    ),
            )
    }

    fn pending_friend_requests(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(bootstrap) = &self.bootstrap else {
            return Vec::new();
        };
        bootstrap
            .friend_requests
            .iter()
            .filter(|request| {
                request.recipient_id == bootstrap.profile.id
                    && request.status == iamrust_domain::FriendRequestStatus::Pending
            })
            .enumerate()
            .map(|(index, request)| {
                let request_id = request.id;
                let sender = bootstrap
                    .friend_request_profiles
                    .iter()
                    .find(|profile| profile.id == request.sender_id)
                    .map_or_else(|| "未知用户".to_owned(), profile_name);
                div()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(sender))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(request.message.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .mt_2()
                            .child(
                                Button::new(("accept-friend", index))
                                    .small()
                                    .primary()
                                    .label("接受")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.decide_friend_request(
                                            request_id,
                                            iamrust_protocol::FriendRequestDecision::Accept,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(("reject-friend", index))
                                    .small()
                                    .outline()
                                    .label("拒绝")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.decide_friend_request(
                                            request_id,
                                            iamrust_protocol::FriendRequestDecision::Reject,
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_contact_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let friends = self.filtered_friends(cx);
        let Some(profile) = friends.get(self.selected_contact).cloned() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .text_color(cx.theme().muted_foreground)
                .child("暂无联系人")
                .into_any_element();
        };
        let user_id = profile.id;
        let delete_label = if self.delete_friend_confirmation == Some(user_id) {
            "再次点击确认删除"
        } else {
            "删除好友"
        };
        v_flex()
            .flex_1()
            .h_full()
            .overflow_y_scrollbar()
            .items_center()
            .justify_center()
            .gap_4()
            .p_8()
            .child(Avatar::new().name(profile_name(&profile)).large())
            .child(
                div()
                    .text_size(px(24.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(profile_name(&profile)),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("@{} · {:?}", profile.username, profile.presence)),
            )
            .child(div().max_w(px(520.)).text_center().child(profile.signature))
            .when_some(self.action_error.clone(), |view, error| {
                view.child(div().text_color(cx.theme().danger).child(error))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("contact-message")
                            .primary()
                            .label("发消息")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_direct_conversation(user_id, cx);
                            })),
                    )
                    .child(
                        Button::new("contact-block")
                            .outline()
                            .label("拉黑")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_blocked(user_id, true, cx);
                            })),
                    )
                    .child(
                        Button::new("contact-report")
                            .outline()
                            .label("举报")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.report_contact(user_id, cx);
                            })),
                    )
                    .child(
                        Button::new("contact-delete")
                            .danger()
                            .label(delete_label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.delete_contact(user_id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_global_search(&self, cx: &mut Context<Self>) -> AnyElement {
        let results = self
            .user_search_results
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, profile)| self.render_user_search_result(index, profile, cx))
            .collect::<Vec<_>>();

        v_flex()
            .flex_1()
            .h_full()
            .items_center()
            .p_8()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(760.))
                    .gap_4()
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("搜索"),
                    )
                    .child(self.render_local_search_panel(cx))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("按用户名查找用户"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&self.user_search).large().cleanable(true))
                            .child(
                                Button::new("search-user")
                                    .primary()
                                    .large()
                                    .loading(self.action_busy)
                                    .label("搜索")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_user_search(cx);
                                    })),
                            ),
                    )
                    .child(Input::new(&self.friend_request_message).large())
                    .when_some(self.action_error.clone(), |view, error| {
                        view.child(div().text_color(cx.theme().danger).child(error))
                    })
                    .when(
                        self.user_search_results.is_empty() && !self.action_busy,
                        |view| {
                            view.child(
                                div()
                                    .py_8()
                                    .text_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("输入完整用户名开始搜索"),
                            )
                        },
                    )
                    .children(results),
            )
            .into_any_element()
    }

    fn render_local_search_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let results = self
            .local_search_results
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, message)| self.render_local_search_result(index, message, cx))
            .collect::<Vec<_>>();
        v_flex()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("本地消息"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child("仅搜索已同步到本机的消息；加密缓存不会建立明文索引。"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Input::new(&self.local_message_search)
                            .large()
                            .cleanable(true),
                    )
                    .child(
                        Button::new("search-local-messages")
                            .primary()
                            .large()
                            .loading(self.local_search_busy)
                            .label("搜索消息")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_local_message_search(cx);
                            })),
                    ),
            )
            .when_some(self.local_search_error.clone(), |view, error| {
                view.child(div().text_color(cx.theme().danger).child(error))
            })
            .children(results)
            .into_any_element()
    }

    fn render_local_search_result(
        &self,
        index: usize,
        message: Message,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let conversation_id = message.conversation_id;
        let conversation_name = self
            .conversations
            .iter()
            .find(|conversation| conversation.id == Some(conversation_id))
            .map_or_else(
                || "未知会话".to_owned(),
                |conversation| conversation.name.clone(),
            );
        let body = match message.content {
            MessageContent::Text { text } | MessageContent::System { text } => text,
            _ => "[不支持的搜索结果]".to_owned(),
        };
        h_flex()
            .w_full()
            .gap_3()
            .p_3()
            .rounded_md()
            .bg(cx.theme().secondary.opacity(0.55))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(format!(
                        "{} · {}",
                        conversation_name,
                        user_display_name(self.bootstrap.as_ref(), message.sender_id)
                    )))
                    .child(div().truncate().child(body))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                message
                                    .server_created_at
                                    .unwrap_or(message.created_at)
                                    .with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string(),
                            ),
                    ),
            )
            .child(
                Button::new(("open-search-message", index))
                    .small()
                    .outline()
                    .label("打开")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_search_conversation(conversation_id, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn start_local_message_search(&mut self, cx: &mut Context<Self>) {
        if self.local_search_busy {
            return;
        }
        let query = self.local_message_search.read(cx).value().trim().to_owned();
        if query.is_empty() || query.chars().count() > 200 {
            self.local_search_error = Some("请输入 1–200 个字符的搜索内容".to_owned());
            cx.notify();
            return;
        }
        let Some(store) = self.store.clone() else {
            self.local_search_error = Some("本地消息缓存不可用".to_owned());
            cx.notify();
            return;
        };
        let runtime = self.runtime.clone();
        self.local_search_busy = true;
        self.local_search_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { runtime.block_on(store.search_messages(&query, 50)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.local_search_busy = false;
                match result {
                    Ok(results) => {
                        this.local_search_results = results;
                        if this.local_search_results.is_empty() {
                            this.local_search_error = Some("本机缓存中没有匹配消息".to_owned());
                        }
                    }
                    Err(error) => this.local_search_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_search_conversation(
        &mut self,
        conversation_id: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .conversations
            .iter()
            .position(|conversation| conversation.id == Some(conversation_id))
        else {
            self.local_search_error = Some("该会话当前不可见".to_owned());
            cx.notify();
            return;
        };
        self.navigation = Navigation::Chats;
        self.select_conversation(index, window, cx);
    }

    fn render_user_search_result(
        &self,
        index: usize,
        profile: iamrust_domain::UserProfile,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let username = profile.username.clone();
        let already_friend = self.bootstrap.as_ref().is_some_and(|bootstrap| {
            bootstrap
                .friends
                .iter()
                .any(|friend| friend.id == profile.id)
        });
        div()
            .w_full()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_3()
                    .child(Avatar::new().name(profile_name(&profile)).large())
                    .child(
                        v_flex()
                            .flex_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(profile_name(&profile)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "@{} · {}",
                                        profile.username, profile.signature
                                    )),
                            ),
                    )
                    .child(
                        Button::new(("add-friend", index))
                            .primary()
                            .disabled(already_friend || self.action_busy)
                            .label(if already_friend {
                                "已是好友"
                            } else {
                                "添加好友"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.send_friend_request(username.clone(), cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn start_user_search(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let username = self.user_search.read(cx).value().trim().to_owned();
        if username.is_empty() {
            self.action_error = Some("请输入用户名".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            self.action_error = Some("服务器地址配置无效".to_owned());
            cx.notify();
            return;
        };
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.search_user(&username) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(results) => {
                        this.user_search_results = results;
                        if this.user_search_results.is_empty() {
                            this.action_error = Some("未找到该用户".to_owned());
                        }
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send_friend_request(&mut self, username: String, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        let message = self.friend_request_message.read(cx).value().to_string();
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.send_friend_request(username, message) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(_) => {
                        this.action_error = Some("好友申请已发送".to_owned());
                        this.refresh_bootstrap(cx);
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn decide_friend_request(
        &mut self,
        request_id: iamrust_domain::FriendRequestId,
        decision: iamrust_protocol::FriendRequestDecision,
        cx: &mut Context<Self>,
    ) {
        if self.action_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.decide_friend_request(request_id, decision) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(_) => this.refresh_bootstrap(cx),
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_direct_conversation(
        &mut self,
        user_id: iamrust_domain::UserId,
        cx: &mut Context<Self>,
    ) {
        if self.action_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.create_direct(user_id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(conversation) => {
                        this.navigation = Navigation::Chats;
                        this.refresh_bootstrap(cx);
                        this.action_error = Some(format!("已打开会话 {}", conversation.id));
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_contact(&mut self, user_id: iamrust_domain::UserId, cx: &mut Context<Self>) {
        if self.delete_friend_confirmation != Some(user_id) {
            self.delete_friend_confirmation = Some(user_id);
            cx.notify();
            return;
        }
        self.delete_friend_confirmation = None;
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.delete_friend(user_id) });
        Self::finish_contact_mutation(task, cx);
    }

    fn set_blocked(
        &mut self,
        user_id: iamrust_domain::UserId,
        blocked: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.block_user(user_id, blocked) });
        Self::finish_contact_mutation(task, cx);
    }

    fn report_contact(&mut self, user_id: iamrust_domain::UserId, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx.background_executor().spawn(async move {
            api.report_user(
                user_id,
                "spam".to_owned(),
                Some("由桌面客户端提交".to_owned()),
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                this.action_error = Some(match result {
                    Ok(()) => "举报已提交".to_owned(),
                    Err(error) => error.user_message(),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_contact_mutation(
        task: gpui::Task<Result<(), crate::api::ClientError>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        this.selected_contact = 0;
                        this.refresh_bootstrap(cx);
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_conversation_details(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = self.selected_conversation_data().cloned() else {
            return v_flex()
                .h_full()
                .w(px(380.))
                .border_l_1()
                .border_color(cx.theme().border)
                .items_center()
                .justify_center()
                .child("请选择会话")
                .into_any_element();
        };
        match conversation.kind {
            ConversationKind::Direct { .. } => self.render_direct_details(conversation, cx),
            ConversationKind::Group { .. } => self.render_group_details(conversation, cx),
        }
    }

    fn render_direct_details(
        &self,
        conversation: Conversation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let members = conversation
            .members
            .values()
            .map(|member| {
                div()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().secondary)
                    .child(user_display_name(self.bootstrap.as_ref(), member.user_id))
            })
            .collect::<Vec<_>>();
        v_flex()
            .h_full()
            .w(px(380.))
            .flex_shrink_0()
            .gap_3()
            .p_4()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("会话详情"),
            )
            .child(self.render_conversation_controls(cx))
            .children(members)
            .into_any_element()
    }

    fn render_group_details(
        &self,
        conversation: Conversation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let role = self
            .bootstrap
            .as_ref()
            .and_then(|bootstrap| conversation.members.get(&bootstrap.profile.id))
            .map_or(MemberRole::Member, |member| member.role);
        let can_manage = role >= MemberRole::Administrator;
        let owner = role == MemberRole::Owner;
        v_flex()
            .h_full()
            .w(px(390.))
            .flex_shrink_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(px(68.))
                    .flex_shrink_0()
                    .justify_between()
                    .px_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(conversation.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} 位成员 · {}",
                                        conversation.members.len(),
                                        role_label(role)
                                    )),
                            ),
                    )
                    .child(
                        Button::new("close-details")
                            .ghost()
                            .small()
                            .label("关闭")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.group_details_open = false;
                                this.reset_group_details();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_4()
                    .p_4()
                    .when(self.group_details_loading, |view| {
                        view.child(
                            div()
                                .text_center()
                                .text_color(cx.theme().muted_foreground)
                                .child("正在加载群资料…"),
                        )
                    })
                    .child(self.render_conversation_controls(cx))
                    .when(can_manage, |view| {
                        view.child(self.render_group_admin_controls(&conversation, cx))
                    })
                    .child(self.render_group_invites(&conversation, can_manage, cx))
                    .child(self.render_group_members(&conversation, role, cx))
                    .child(self.render_group_announcements(can_manage, cx))
                    .child(self.render_group_join_requests(can_manage, cx))
                    .child(self.render_group_polls(can_manage, cx))
                    .child(self.render_group_files(cx))
                    .child(self.render_group_exit_actions(owner, cx)),
            )
            .into_any_element()
    }

    fn render_group_admin_controls(
        &self,
        conversation: &Conversation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("群设置"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("当前名称：{}", conversation.name)),
            )
            .child(Input::new(&self.group_edit_name).small().cleanable(true))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("save-group-name")
                            .small()
                            .primary()
                            .disabled(self.action_busy)
                            .label("保存名称")
                            .on_click(cx.listener(|this, _, _, cx| this.save_group_name(cx))),
                    )
                    .child(
                        Button::new("group-mute-all")
                            .small()
                            .outline()
                            .selected(self.group_mute_all)
                            .disabled(self.action_busy)
                            .label(if self.group_mute_all {
                                "解除全员禁言"
                            } else {
                                "全员禁言"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_group_mute_all(cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_conversation_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.selected_conversation_data().and_then(|conversation| {
            self.bootstrap
                .as_ref()?
                .conversation_states
                .iter()
                .find(|state| state.conversation_id == conversation.id)
        });
        let pinned = state.is_some_and(|state| state.pinned);
        let muted = state.map_or_else(
            || {
                self.conversations
                    .get(self.selected_conversation)
                    .is_some_and(|preview| preview.muted)
            },
            |state| state.muted,
        );
        let manually_unread = state.is_some_and(|state| state.manually_unread);
        h_flex()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("conversation-pin")
                    .small()
                    .outline()
                    .selected(pinned)
                    .disabled(self.action_busy)
                    .label(if pinned { "取消置顶" } else { "置顶" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_selected_conversation_settings(
                            UpdateConversationSettingsRequest {
                                pinned: Some(!pinned),
                                ..Default::default()
                            },
                            "会话置顶状态已更新",
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("conversation-mute")
                    .small()
                    .outline()
                    .selected(muted)
                    .disabled(self.action_busy)
                    .label(if muted {
                        "关闭免打扰"
                    } else {
                        "免打扰"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_selected_conversation_settings(
                            UpdateConversationSettingsRequest {
                                muted: Some(!muted),
                                ..Default::default()
                            },
                            "免打扰状态已更新",
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("conversation-unread")
                    .small()
                    .outline()
                    .selected(manually_unread)
                    .disabled(self.action_busy)
                    .label(if manually_unread {
                        "取消未读"
                    } else {
                        "标为未读"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_selected_conversation_settings(
                            UpdateConversationSettingsRequest {
                                manually_unread: Some(!manually_unread),
                                ..Default::default()
                            },
                            "未读状态已更新",
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("conversation-hide")
                    .small()
                    .danger()
                    .disabled(self.action_busy)
                    .label("隐藏会话")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.update_selected_conversation_settings(
                            UpdateConversationSettingsRequest {
                                hidden: Some(true),
                                ..Default::default()
                            },
                            "会话已隐藏，新消息到达后会重新出现",
                            cx,
                        );
                    })),
            )
            .into_any_element()
    }

    fn update_selected_conversation_settings(
        &mut self,
        request: UpdateConversationSettingsRequest,
        success_message: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.action_busy {
            return;
        }
        let Some(conversation_id) = self.selected_conversation_data().map(|item| item.id) else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.update_conversation_settings(conversation_id, request) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        this.message_error = Some(success_message.to_owned());
                        this.refresh_bootstrap(cx);
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn mark_all_conversations_read(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.mark_all_read() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        for conversation in &mut this.conversations {
                            conversation.unread = 0;
                        }
                        this.refresh_bootstrap(cx);
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_group_invites(
        &self,
        conversation: &Conversation,
        can_manage: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let candidates = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.friends.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|profile| !conversation.members.contains_key(&profile.id))
            .collect::<Vec<_>>();
        let chips = candidates.into_iter().enumerate().map(|(index, profile)| {
            let user_id = profile.id;
            Button::new(("group-invite", index))
                .small()
                .selected(self.group_invite_members.contains(&user_id))
                .label(profile_name(&profile))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.group_invite_members.insert(user_id) {
                        this.group_invite_members.remove(&user_id);
                    }
                    cx.notify();
                }))
        });
        v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("邀请好友"),
            )
            .when(!can_manage, |view| {
                view.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child("只有群主和管理员可以邀请成员。"),
                )
            })
            .when(can_manage, |view| {
                view.child(h_flex().flex_wrap().gap_1().children(chips))
                    .child(
                        Button::new("invite-group-members")
                            .small()
                            .outline()
                            .disabled(self.group_invite_members.is_empty() || self.action_busy)
                            .label(format!("邀请 {} 人", self.group_invite_members.len()))
                            .on_click(cx.listener(|this, _, _, cx| this.invite_group_members(cx))),
                    )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)]
    fn render_group_members(
        &self,
        conversation: &Conversation,
        actor_role: MemberRole,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_user_id = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.profile.id);
        let rows = conversation
            .members
            .values()
            .cloned()
            .enumerate()
            .map(|(index, member)| {
                let target_id = member.user_id;
                let target_role = member.role;
                let is_self = current_user_id == Some(target_id);
                let can_manage =
                    !is_self && actor_role >= MemberRole::Administrator && actor_role > target_role;
                let muted = member.muted_until.is_some_and(|until| until > Utc::now());
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().secondary.opacity(0.55))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(user_display_name(self.bootstrap.as_ref(), target_id)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(role_label(target_role)),
                            ),
                    )
                    .when(can_manage, |row| {
                        row.child(
                            h_flex()
                                .flex_wrap()
                                .gap_1()
                                .when(actor_role == MemberRole::Owner, |actions| {
                                    actions.child(
                                        Button::new(("group-role", index))
                                            .small()
                                            .ghost()
                                            .label(if target_role == MemberRole::Administrator {
                                                "取消管理员"
                                            } else {
                                                "设为管理员"
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let role =
                                                    if target_role == MemberRole::Administrator {
                                                        MemberRole::Member
                                                    } else {
                                                        MemberRole::Administrator
                                                    };
                                                this.set_group_member_role(target_id, role, cx);
                                            })),
                                    )
                                })
                                .child(
                                    Button::new(("group-member-mute", index))
                                        .small()
                                        .ghost()
                                        .label(if muted { "解除禁言" } else { "禁言 24h" })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_group_member_muted(target_id, !muted, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("group-member-remove", index))
                                        .small()
                                        .danger()
                                        .label(
                                            if self.group_confirmation
                                                == Some(GroupConfirmation::Remove(target_id))
                                            {
                                                "再次确认移除"
                                            } else {
                                                "移除"
                                            },
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.remove_group_member(target_id, cx);
                                        })),
                                )
                                .when(actor_role == MemberRole::Owner, |actions| {
                                    actions.child(
                                        Button::new(("group-transfer", index))
                                            .small()
                                            .danger()
                                            .label(
                                                if self.group_confirmation
                                                    == Some(GroupConfirmation::Transfer(target_id))
                                                {
                                                    "再次确认转让"
                                                } else {
                                                    "转让群主"
                                                },
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.transfer_group(target_id, cx);
                                            })),
                                    )
                                }),
                        )
                    })
            });
        v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("群成员（{}）", conversation.members.len())),
            )
            .children(rows)
            .into_any_element()
    }

    fn render_group_announcements(&self, can_manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let current_user_id = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.profile.id);
        let rows = self
            .group_announcements
            .iter()
            .rev()
            .cloned()
            .enumerate()
            .map(|(index, announcement)| {
                let announcement_id = announcement.id;
                let unread =
                    current_user_id.is_some_and(|user_id| !announcement.read_by.contains(&user_id));
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().secondary.opacity(0.55))
                    .child(div().child(announcement.content))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {}",
                                        user_display_name(
                                            self.bootstrap.as_ref(),
                                            announcement.author_id
                                        ),
                                        announcement
                                            .created_at
                                            .with_timezone(&chrono::Local)
                                            .format("%m-%d %H:%M")
                                    )),
                            )
                            .when(unread, |meta| {
                                meta.child(
                                    Button::new(("read-announcement", index))
                                        .small()
                                        .ghost()
                                        .label("标为已读")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.read_group_announcement(announcement_id, cx);
                                        })),
                                )
                            }),
                    )
            });
        v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("群公告"),
            )
            .when(can_manage, |view| {
                view.child(
                    Input::new(&self.group_announcement_input)
                        .small()
                        .cleanable(true),
                )
                .child(
                    Button::new("publish-announcement")
                        .small()
                        .outline()
                        .disabled(self.action_busy)
                        .label("发布公告")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.create_group_announcement(cx);
                        })),
                )
            })
            .when(self.group_announcements.is_empty(), |view| {
                view.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child("暂无群公告"),
                )
            })
            .children(rows)
            .into_any_element()
    }

    fn render_group_join_requests(&self, can_manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let pending = self
            .group_join_requests
            .iter()
            .filter(|request| request.status == GroupJoinRequestStatus::Pending)
            .cloned()
            .collect::<Vec<_>>();
        let rows = pending.iter().cloned().enumerate().map(|(index, request)| {
            let request_id = request.id;
            v_flex()
                .gap_2()
                .p_3()
                .rounded_md()
                .bg(cx.theme().secondary.opacity(0.55))
                .child(format!(
                    "{}：{}",
                    user_display_name(self.bootstrap.as_ref(), request.applicant_id),
                    request.message
                ))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new(("accept-join", index))
                                .small()
                                .primary()
                                .label("同意")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide_group_join_request(request_id, true, cx);
                                })),
                        )
                        .child(
                            Button::new(("reject-join", index))
                                .small()
                                .danger()
                                .label("拒绝")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide_group_join_request(request_id, false, cx);
                                })),
                        ),
                )
        });
        v_flex()
            .gap_2()
            .when(can_manage && !pending.is_empty(), |view| {
                view.child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!("入群申请（{}）", pending.len())),
                )
                .children(rows)
            })
            .into_any_element()
    }

    fn render_group_polls(&self, can_manage: bool, cx: &mut Context<Self>) -> AnyElement {
        let polls = self
            .group_polls
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, poll)| self.render_group_poll(index, poll, cx))
            .collect::<Vec<_>>();
        v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("群投票"),
            )
            .when(can_manage, |view| {
                view.child(
                    Input::new(&self.group_poll_question)
                        .small()
                        .cleanable(true),
                )
                .child(
                    Input::new(&self.group_poll_option_a)
                        .small()
                        .cleanable(true),
                )
                .child(
                    Input::new(&self.group_poll_option_b)
                        .small()
                        .cleanable(true),
                )
                .child(
                    Button::new("create-group-poll")
                        .small()
                        .outline()
                        .disabled(self.action_busy)
                        .label("创建单选投票")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.create_group_poll(cx);
                        })),
                )
            })
            .when(self.group_polls.is_empty(), |view| {
                view.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child("暂无投票"),
                )
            })
            .children(polls)
            .into_any_element()
    }

    fn render_group_poll(
        &self,
        index: usize,
        poll: GroupPoll,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let poll_id = poll.id;
        let multiple_choice = poll.multiple_choice;
        let option_ids = poll
            .options
            .iter()
            .map(|option| option.id)
            .collect::<Vec<_>>();
        let selected_count = option_ids
            .iter()
            .filter(|option_id| self.selected_poll_options.contains(option_id))
            .count();
        let options = poll
            .options
            .into_iter()
            .enumerate()
            .map(|(option_index, option)| {
                let option_id = option.id;
                let all_option_ids = option_ids.clone();
                Button::new(("poll-option", index * 100 + option_index))
                    .w_full()
                    .small()
                    .outline()
                    .selected(self.selected_poll_options.contains(&option_id))
                    .label(format!("{} · {} 票", option.label, option.voter_ids.len()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_poll_option(&all_option_ids, option_id, multiple_choice, cx);
                    }))
            });
        v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .bg(cx.theme().secondary.opacity(0.55))
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(poll.question),
            )
            .children(options)
            .child(
                Button::new(("submit-poll", index))
                    .small()
                    .primary()
                    .disabled(selected_count == 0 || self.action_busy)
                    .label("提交投票")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit_poll_vote(poll_id, &option_ids, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_group_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let files = self.group_files.iter().cloned().map(|item| {
            h_flex()
                .justify_between()
                .gap_2()
                .p_2()
                .rounded_md()
                .bg(cx.theme().secondary.opacity(0.55))
                .child(div().truncate().child(item.attachment.file_name))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child(format_file_size(item.attachment.byte_size)),
                )
        });
        v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("群文件（{}）", self.group_files.len())),
            )
            .when(self.group_files.is_empty(), |view| {
                view.child(
                    div()
                        .text_size(px(11.))
                        .text_color(cx.theme().muted_foreground)
                        .child("暂无群文件"),
                )
            })
            .children(files)
            .into_any_element()
    }

    fn render_group_exit_actions(&self, owner: bool, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("leave-or-disband-group")
                    .danger()
                    .disabled(self.action_busy)
                    .label(if owner {
                        if self.group_confirmation == Some(GroupConfirmation::Disband) {
                            "再次点击确认解散群聊"
                        } else {
                            "解散群聊"
                        }
                    } else if self.group_confirmation == Some(GroupConfirmation::Leave) {
                        "再次点击确认退出群聊"
                    } else {
                        "退出群聊"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.leave_or_disband_group(owner, cx);
                    })),
            )
            .into_any_element()
    }

    fn selected_group_id(&self) -> Option<ConversationId> {
        self.selected_conversation_data().and_then(|conversation| {
            matches!(conversation.kind, ConversationKind::Group { .. }).then_some(conversation.id)
        })
    }

    fn apply_group_conversation(&mut self, conversation: Conversation, cx: &mut Context<Self>) {
        if let Some(bootstrap) = &mut self.bootstrap
            && let Some(existing) = bootstrap
                .conversations
                .iter_mut()
                .find(|existing| existing.id == conversation.id)
        {
            existing.clone_from(&conversation);
        }
        if let Some(preview) = self
            .conversations
            .iter_mut()
            .find(|preview| preview.id == Some(conversation.id))
        {
            preview.name.clone_from(&conversation.name);
        }
        if let Some(bootstrap) = self.bootstrap.clone() {
            self.persist_bootstrap(bootstrap, cx);
        }
    }

    fn save_group_name(&mut self, cx: &mut Context<Self>) {
        let name = self.group_edit_name.read(cx).value().trim().to_owned();
        if name.is_empty() || name.chars().count() > 80 {
            self.message_error = Some("群名称应为 1–80 个字符".to_owned());
            cx.notify();
            return;
        }
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.update_group(conversation_id, Some(name)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(conversation) => {
                        this.apply_group_conversation(conversation, cx);
                        this.message_error = Some("群名称已更新".to_owned());
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_group_mute_all(&mut self, cx: &mut Context<Self>) {
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let muted = !self.group_mute_all;
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.set_group_mute(conversation_id, muted) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => this.group_mute_all = muted,
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn invite_group_members(&mut self, cx: &mut Context<Self>) {
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let member_ids = self
            .group_invite_members
            .iter()
            .copied()
            .collect::<Vec<_>>();
        if member_ids.is_empty() {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.add_group_members(conversation_id, member_ids) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(conversation) => {
                        this.group_invite_members.clear();
                        this.apply_group_conversation(conversation, cx);
                        this.message_error = Some("好友已加入群聊".to_owned());
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_group_member_role(
        &mut self,
        member_id: UserId,
        role: MemberRole,
        cx: &mut Context<Self>,
    ) {
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.update_group_member_role(conversation_id, member_id, role) });
        Self::finish_group_conversation_mutation(task, cx);
    }

    fn set_group_member_muted(&mut self, member_id: UserId, muted: bool, cx: &mut Context<Self>) {
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let muted_until = if muted {
            Some(Utc::now() + chrono::Duration::hours(24))
        } else {
            None
        };
        self.action_busy = true;
        let task = cx.background_executor().spawn(async move {
            api.update_group_member_mute(conversation_id, member_id, muted_until)
        });
        Self::finish_group_conversation_mutation(task, cx);
    }

    fn finish_group_conversation_mutation(
        task: gpui::Task<Result<Conversation, ClientError>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(conversation) => this.apply_group_conversation(conversation, cx),
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_group_member(&mut self, member_id: UserId, cx: &mut Context<Self>) {
        let confirmation = GroupConfirmation::Remove(member_id);
        if self.group_confirmation != Some(confirmation) {
            self.group_confirmation = Some(confirmation);
            cx.notify();
            return;
        }
        self.group_confirmation = None;
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.remove_group_member(conversation_id, member_id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        if let Some(bootstrap) = &mut this.bootstrap
                            && let Some(conversation) = bootstrap
                                .conversations
                                .iter_mut()
                                .find(|conversation| conversation.id == conversation_id)
                        {
                            conversation.members.remove(&member_id);
                        }
                        if let Some(bootstrap) = this.bootstrap.clone() {
                            this.persist_bootstrap(bootstrap, cx);
                        }
                        this.message_error = Some("成员已移除".to_owned());
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn transfer_group(&mut self, member_id: UserId, cx: &mut Context<Self>) {
        let confirmation = GroupConfirmation::Transfer(member_id);
        if self.group_confirmation != Some(confirmation) {
            self.group_confirmation = Some(confirmation);
            cx.notify();
            return;
        }
        self.group_confirmation = None;
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.transfer_group(conversation_id, member_id) });
        Self::finish_group_conversation_mutation(task, cx);
    }

    fn create_group_announcement(&mut self, cx: &mut Context<Self>) {
        let content = self
            .group_announcement_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if content.is_empty() {
            self.message_error = Some("请输入群公告".to_owned());
            cx.notify();
            return;
        }
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.create_group_announcement(conversation_id, content) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(announcement) => {
                        this.group_announcements.push(announcement);
                        this.message_error = Some("群公告已发布".to_owned());
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn read_group_announcement(&mut self, announcement_id: Uuid, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let current_user_id = self
            .bootstrap
            .as_ref()
            .map(|bootstrap| bootstrap.profile.id);
        let task = cx
            .background_executor()
            .spawn(async move { api.read_group_announcement(announcement_id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        if let (Some(user_id), Some(announcement)) = (
                            current_user_id,
                            this.group_announcements
                                .iter_mut()
                                .find(|announcement| announcement.id == announcement_id),
                        ) && !announcement.read_by.contains(&user_id)
                        {
                            announcement.read_by.push(user_id);
                        }
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn decide_group_join_request(
        &mut self,
        request_id: Uuid,
        accept: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.decide_group_join_request(request_id, accept) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(updated) => {
                        if let Some(request) = this
                            .group_join_requests
                            .iter_mut()
                            .find(|request| request.id == request_id)
                        {
                            *request = updated;
                        }
                        if accept {
                            this.refresh_bootstrap(cx);
                        }
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create_group_poll(&mut self, cx: &mut Context<Self>) {
        let question = self.group_poll_question.read(cx).value().trim().to_owned();
        let option_a = self.group_poll_option_a.read(cx).value().trim().to_owned();
        let option_b = self.group_poll_option_b.read(cx).value().trim().to_owned();
        if question.is_empty() || option_a.is_empty() || option_b.is_empty() {
            self.message_error = Some("请填写投票问题和两个选项".to_owned());
            cx.notify();
            return;
        }
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let request = CreateGroupPollRequest {
            question,
            options: vec![option_a, option_b],
            multiple_choice: false,
            closes_at: None,
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.create_group_poll(conversation_id, request) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(poll) => this.group_polls.push(poll),
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_poll_option(
        &mut self,
        poll_options: &[Uuid],
        option_id: Uuid,
        multiple_choice: bool,
        cx: &mut Context<Self>,
    ) {
        let selected = self.selected_poll_options.contains(&option_id);
        if !multiple_choice {
            self.selected_poll_options
                .retain(|selected_id| !poll_options.contains(selected_id));
        }
        if selected {
            self.selected_poll_options.remove(&option_id);
        } else {
            self.selected_poll_options.insert(option_id);
        }
        cx.notify();
    }

    fn submit_poll_vote(&mut self, poll_id: Uuid, poll_options: &[Uuid], cx: &mut Context<Self>) {
        let option_ids = poll_options
            .iter()
            .filter(|option_id| self.selected_poll_options.contains(option_id))
            .copied()
            .collect::<Vec<_>>();
        if option_ids.is_empty() {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.vote_group_poll(poll_id, option_ids) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(poll) => {
                        let option_ids = poll
                            .options
                            .iter()
                            .map(|option| option.id)
                            .collect::<Vec<_>>();
                        this.selected_poll_options
                            .retain(|option_id| !option_ids.contains(option_id));
                        if let Some(existing) = this
                            .group_polls
                            .iter_mut()
                            .find(|existing| existing.id == poll_id)
                        {
                            *existing = poll;
                        }
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn leave_or_disband_group(&mut self, owner: bool, cx: &mut Context<Self>) {
        let confirmation = if owner {
            GroupConfirmation::Disband
        } else {
            GroupConfirmation::Leave
        };
        if self.group_confirmation != Some(confirmation) {
            self.group_confirmation = Some(confirmation);
            cx.notify();
            return;
        }
        self.group_confirmation = None;
        let Some(conversation_id) = self.selected_group_id() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx.background_executor().spawn(async move {
            if owner {
                api.disband_group(conversation_id)
            } else {
                api.leave_group(conversation_id)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        this.group_details_open = false;
                        this.reset_group_details();
                        this.refresh_bootstrap(cx);
                    }
                    Err(error) => this.message_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_security_settings(&mut self, cx: &mut Context<Self>) {
        if self.security_loading || self.bootstrap.is_none() {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.security_loading = true;
        let task = cx.background_executor().spawn(async move {
            let devices = api.devices()?;
            let second_factor = api.second_factor_status()?;
            Ok::<_, ClientError>((devices, second_factor))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.security_loading = false;
                match result {
                    Ok((devices, status)) => {
                        this.devices = devices;
                        this.second_factor_status = Some(status);
                        this.security_loaded = true;
                    }
                    Err(error) => {
                        this.security_loaded = false;
                        this.action_error = Some(error.user_message());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn begin_second_factor_setup(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.begin_second_factor_setup() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(setup) => this.second_factor_setup = Some(setup),
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn enable_second_factor(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let code = self.security_code.read(cx).value().trim().to_owned();
        if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
            self.action_error = Some("请输入身份验证器中的 6 位验证码".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.enable_second_factor(&code) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(response) => {
                        this.second_factor_status = Some(SecondFactorStatus {
                            enabled: true,
                            recovery_codes_remaining: response.recovery_codes.len(),
                        });
                        this.second_factor_setup = None;
                        this.recovery_codes = response.recovery_codes;
                        this.action_error = Some("双因素认证已启用，请立即保存恢复码。".to_owned());
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn disable_second_factor(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let password = self.second_factor_password.read(cx).value().to_string();
        let code = self.security_code.read(cx).value().trim().to_owned();
        if password.is_empty() || code.is_empty() {
            self.action_error = Some("请输入当前密码以及验证码或恢复码".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.disable_second_factor(&password, &code) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        this.second_factor_status = Some(SecondFactorStatus {
                            enabled: false,
                            recovery_codes_remaining: 0,
                        });
                        this.recovery_codes.clear();
                        this.action_error = Some("双因素认证已关闭。".to_owned());
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn regenerate_recovery_codes(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let password = self.second_factor_password.read(cx).value().to_string();
        let code = self.security_code.read(cx).value().trim().to_owned();
        if password.is_empty() || code.is_empty() {
            self.action_error = Some("请输入当前密码以及验证码或恢复码".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.regenerate_recovery_codes(&password, &code) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(response) => {
                        let count = response.recovery_codes.len();
                        this.recovery_codes = response.recovery_codes;
                        this.second_factor_status = Some(SecondFactorStatus {
                            enabled: true,
                            recovery_codes_remaining: count,
                        });
                        this.action_error = Some("新恢复码已生成，旧恢复码已失效。".to_owned());
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn approve_qr_login(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let payload = self.qr_approval_payload.read(cx).value().trim().to_owned();
        if payload.is_empty() {
            self.action_error = Some("请粘贴二维码安全载荷".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.approve_qr_payload(&payload) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                this.action_error = Some(match result {
                    Ok(()) => "已批准新设备登录。".to_owned(),
                    Err(error) => error.user_message(),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn revoke_device(&mut self, device_id: iamrust_domain::DeviceId, cx: &mut Context<Self>) {
        if self.revoke_device_confirmation != Some(device_id) {
            self.revoke_device_confirmation = Some(device_id);
            cx.notify();
            return;
        }
        self.revoke_device_confirmation = None;
        let Some(api) = self.api.clone() else {
            return;
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.revoke_device(device_id) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        this.devices.retain(|device| device.id != device_id);
                        this.action_error = Some("远程设备会话已撤销。".to_owned());
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn change_account_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let current_password = self.security_current_password.read(cx).value().to_string();
        let new_password = self.security_new_password.read(cx).value().to_string();
        let confirmation = self.security_confirm_password.read(cx).value().to_string();
        if current_password.is_empty() || !(10..=128).contains(&new_password.len()) {
            self.action_error = Some("请输入当前密码；新密码长度应为 10–128 位".to_owned());
            cx.notify();
            return;
        }
        if new_password != confirmation {
            self.action_error = Some("两次输入的新密码不一致".to_owned());
            cx.notify();
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        for input in [
            &self.security_current_password,
            &self.security_new_password,
            &self.security_confirm_password,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.action_busy = true;
        self.action_error = None;
        let task = cx
            .background_executor()
            .spawn(async move { api.change_password(&current_password, &new_password) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(()) => {
                        this.start_logout(cx);
                        this.auth_notice = Some("密码已修改，请重新登录。".to_owned());
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_settings(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .flex_1()
            .h_full()
            .overflow_y_scrollbar()
            .items_center()
            .p_8()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(820.))
                    .gap_5()
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("设置"),
                    )
                    .when_some(self.action_error.clone(), |view, error| {
                        view.child(div().text_color(cx.theme().danger).child(error))
                    })
                    .child(self.render_profile_settings(cx))
                    .child(self.render_security_settings(cx))
                    .child(self.render_appearance_settings(cx))
                    .child(self.render_local_data_settings(cx)),
            )
            .into_any_element()
    }

    fn settings_card(title: &'static str, cx: &Context<Self>) -> gpui::Div {
        v_flex()
            .w_full()
            .gap_3()
            .p_5()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title),
            )
    }

    fn render_profile_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let profile = self.bootstrap.as_ref().map(|bootstrap| &bootstrap.profile);
        Self::settings_card("账号资料", cx)
            .when_some(profile, |card, profile| {
                card.child(div().text_color(cx.theme().muted_foreground).child(format!(
                    "@{} · 当前昵称：{}",
                    profile.username,
                    profile_name(profile)
                )))
            })
            .child(Input::new(&self.profile_nickname).large().cleanable(true))
            .child(Input::new(&self.profile_signature).large().cleanable(true))
            .child(
                Button::new("save-profile")
                    .primary()
                    .loading(self.action_busy)
                    .label("保存资料")
                    .on_click(cx.listener(|this, _, _, cx| this.save_profile(cx))),
            )
            .into_any_element()
    }

    fn render_security_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        Self::settings_card("隐私与安全", cx)
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("访问令牌只保存在内存，刷新令牌由操作系统安全凭据库管理。"),
            )
            .child(self.render_second_factor_settings(cx))
            .child(self.render_qr_approval(cx))
            .child(self.render_devices(cx))
            .child(
                v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("修改密码"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child("修改成功后，所有设备都需要重新登录。"),
                    )
                    .child(
                        Input::new(&self.security_current_password)
                            .large()
                            .mask_toggle(),
                    )
                    .child(
                        Input::new(&self.security_new_password)
                            .large()
                            .mask_toggle(),
                    )
                    .child(
                        Input::new(&self.security_confirm_password)
                            .large()
                            .mask_toggle(),
                    )
                    .child(
                        Button::new("change-password")
                            .danger()
                            .disabled(self.action_busy)
                            .label("修改密码并退出")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_account_password(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_second_factor_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.security_loading {
            div()
                .text_color(cx.theme().muted_foreground)
                .child("正在加载安全状态…")
                .into_any_element()
        } else if self
            .second_factor_status
            .as_ref()
            .is_some_and(|status| status.enabled)
        {
            let remaining = self
                .second_factor_status
                .as_ref()
                .map_or(0, |status| status.recovery_codes_remaining);
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_color(cx.theme().primary)
                        .child(format!("已启用 · 剩余 {remaining} 枚恢复码")),
                )
                .child(
                    Input::new(&self.second_factor_password)
                        .large()
                        .mask_toggle(),
                )
                .child(Input::new(&self.security_code).large().cleanable(true))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("regenerate-recovery-codes")
                                .outline()
                                .disabled(self.action_busy)
                                .label("重新生成恢复码")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.regenerate_recovery_codes(cx);
                                })),
                        )
                        .child(
                            Button::new("disable-second-factor")
                                .danger()
                                .disabled(self.action_busy)
                                .label("关闭双因素认证")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.disable_second_factor(cx);
                                })),
                        ),
                )
                .into_any_element()
        } else if let Some(setup) = &self.second_factor_setup {
            v_flex()
                .gap_2()
                .child("请在身份验证器中添加以下密钥，然后输入 6 位验证码：")
                .child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(cx.theme().secondary)
                        .font_family("monospace")
                        .child(setup.secret.clone()),
                )
                .child(Input::new(&self.security_code).large().cleanable(true))
                .child(
                    Button::new("enable-second-factor")
                        .primary()
                        .disabled(self.action_busy)
                        .label("验证并启用")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.enable_second_factor(cx);
                        })),
                )
                .into_any_element()
        } else {
            Button::new("begin-second-factor")
                .outline()
                .disabled(self.action_busy || self.bootstrap.is_none())
                .label("启用双因素认证")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.begin_second_factor_setup(cx);
                }))
                .into_any_element()
        }
    }

    fn render_second_factor_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = self.render_second_factor_controls(cx);
        let recovery_codes = self.recovery_codes.iter().cloned().map(|code| {
            div()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(cx.theme().secondary)
                .font_family("monospace")
                .child(code)
        });
        v_flex()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("双因素认证"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(cx.theme().muted_foreground)
                    .child("登录时要求身份验证器验证码；每枚恢复码只能使用一次。"),
            )
            .child(content)
            .when(!self.recovery_codes.is_empty(), |view| {
                view.child(
                    v_flex()
                        .gap_1()
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().primary)
                        .child("仅显示这一次，请立即离线保存")
                        .children(recovery_codes),
                )
            })
            .into_any_element()
    }

    fn render_qr_approval(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("批准扫码登录"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(cx.theme().muted_foreground)
                    .child("粘贴另一台设备二维码中的安全载荷进行确认。"),
            )
            .child(
                Input::new(&self.qr_approval_payload)
                    .large()
                    .cleanable(true),
            )
            .child(
                Button::new("approve-qr-login")
                    .outline()
                    .disabled(self.action_busy || self.bootstrap.is_none())
                    .label("批准登录")
                    .on_click(cx.listener(|this, _, _, cx| this.approve_qr_login(cx))),
            )
            .into_any_element()
    }

    fn render_devices(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self
            .devices
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, device)| {
                let device_id = device.id;
                let button_label = if device.current {
                    "当前设备"
                } else if self.revoke_device_confirmation == Some(device_id) {
                    "再次点击确认"
                } else {
                    "远程退出"
                };
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .p_3()
                    .rounded_md()
                    .bg(cx.theme().secondary.opacity(0.55))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(device.name),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {} · 最近使用 {}",
                                        device.platform,
                                        device.app_version,
                                        device
                                            .last_seen_at
                                            .with_timezone(&chrono::Local)
                                            .format("%Y-%m-%d %H:%M")
                                    )),
                            ),
                    )
                    .child(
                        Button::new(("revoke-device", index))
                            .small()
                            .outline()
                            .disabled(device.current || self.action_busy)
                            .label(button_label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.revoke_device(device_id, cx);
                            })),
                    )
            });
        v_flex()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("登录设备"),
            )
            .when(self.devices.is_empty() && !self.security_loading, |view| {
                view.child(
                    div()
                        .text_size(px(12.))
                        .text_color(cx.theme().muted_foreground)
                        .child("没有可显示的远程设备。"),
                )
            })
            .children(rows)
            .into_any_element()
    }

    fn render_appearance_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        Self::settings_card("外观与通知", cx)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("theme-system")
                            .outline()
                            .selected(self.theme_preference == "system")
                            .label("跟随系统")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_theme("system", window, cx);
                            })),
                    )
                    .child(
                        Button::new("theme-light")
                            .outline()
                            .selected(self.theme_preference == "light")
                            .label("浅色")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_theme("light", window, cx);
                            })),
                    )
                    .child(
                        Button::new("theme-dark")
                            .outline()
                            .selected(self.theme_preference == "dark")
                            .label("深色")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_theme("dark", window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("notifications-toggle")
                            .selected(self.notifications_enabled)
                            .label(if self.notifications_enabled {
                                "通知：开"
                            } else {
                                "通知：关"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notifications_enabled = !this.notifications_enabled;
                                this.persist_bool_setting(
                                    "notifications.enabled",
                                    this.notifications_enabled,
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("sounds-toggle")
                            .selected(self.sounds_enabled)
                            .label(if self.sounds_enabled {
                                "声音：开"
                            } else {
                                "声音：关"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sounds_enabled = !this.sounds_enabled;
                                this.persist_bool_setting(
                                    "sounds.enabled",
                                    this.sounds_enabled,
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("privacy-toggle")
                            .selected(self.privacy_mode)
                            .label(if self.privacy_mode {
                                "隐私预览：开"
                            } else {
                                "隐私预览：关"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.privacy_mode = !this.privacy_mode;
                                this.persist_bool_setting(
                                    "notifications.privacy",
                                    this.privacy_mode,
                                    cx,
                                );
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_local_data_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let stats = self.cache_stats.as_ref().map_or_else(
            || "正在读取缓存统计…".to_owned(),
            |stats| {
                format!(
                    "数据库 {} KiB · {} 条消息 · {} 条待发送",
                    stats.database_bytes / 1024,
                    stats.message_count,
                    stats.pending_outbox_count
                )
            },
        );
        Self::settings_card("本地数据", cx)
            .child(div().text_color(cx.theme().muted_foreground).child(stats))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("cache-encryption")
                            .outline()
                            .label(if self.cache_encryption == Some(true) {
                                "关闭本地加密"
                            } else {
                                "开启本地加密"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_cache_encryption(cx);
                            })),
                    )
                    .child(
                        Button::new("retain-cache")
                            .selected(self.retain_cache_on_logout)
                            .label(if self.retain_cache_on_logout {
                                "退出时保留缓存"
                            } else {
                                "退出时清除缓存"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.retain_cache_on_logout = !this.retain_cache_on_logout;
                                this.persist_bool_setting(
                                    "cache.retain_on_logout",
                                    this.retain_cache_on_logout,
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("clear-cache")
                            .danger()
                            .label(if self.clear_cache_confirmation {
                                "再次点击确认清理"
                            } else {
                                "清理账号缓存"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_local_cache(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "I Am Rust {} · 协议 v{} · MIT OR Apache-2.0",
                        env!("CARGO_PKG_VERSION"),
                        iamrust_protocol::WS_PROTOCOL_VERSION
                    )),
            )
            .into_any_element()
    }

    fn load_local_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        let task = cx.background_executor().spawn(async move {
            runtime.block_on(async {
                Ok::<_, String>((
                    store.cache_stats().await?,
                    store.encryption_enabled().await,
                    store
                        .load_setting::<bool>("cache.retain_on_logout")
                        .await?
                        .unwrap_or(true),
                    store
                        .load_setting::<bool>("notifications.enabled")
                        .await?
                        .unwrap_or(true),
                    store
                        .load_setting::<bool>("sounds.enabled")
                        .await?
                        .unwrap_or(true),
                    store
                        .load_setting::<bool>("notifications.privacy")
                        .await?
                        .unwrap_or(false),
                ))
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((stats, encrypted, retain, notifications, sounds, privacy)) => {
                        this.cache_stats = Some(stats);
                        this.cache_encryption = Some(encrypted);
                        this.retain_cache_on_logout = retain;
                        this.notifications_enabled = notifications;
                        this.sounds_enabled = sounds;
                        this.privacy_mode = privacy;
                    }
                    Err(error) => this.cache_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn schedule_window_placement_save(&self, window: &Window, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let placement = WindowPlacement::capture(window);
        let runtime = self.runtime.clone();
        let revisions = self.window_placement_revision.clone();
        let revision = revisions.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        cx.background_executor()
            .spawn(async move {
                Timer::after(Duration::from_millis(400)).await;
                if revisions.load(Ordering::SeqCst) == revision {
                    let _ =
                        runtime.block_on(store.save_setting(WINDOW_PLACEMENT_SETTING, &placement));
                }
            })
            .detach();
    }

    fn persist_bool_setting(&self, key: &'static str, value: bool, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = runtime.block_on(store.save_setting(key, &value));
            })
            .detach();
    }

    fn change_theme(&mut self, mode: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let theme = match mode {
            "dark" => ThemeMode::Dark,
            "light" => ThemeMode::Light,
            _ => window.appearance().into(),
        };
        self.theme_preference = mode.to_owned();
        Theme::change(theme, Some(window), cx);
        if let Some(store) = self.store.clone() {
            let runtime = self.runtime.clone();
            cx.background_executor()
                .spawn(async move {
                    let _ = runtime.block_on(store.save_setting("ui.theme", mode));
                })
                .detach();
        }
        cx.notify();
    }

    fn save_profile(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let Some(bootstrap) = &self.bootstrap else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let nickname = self.profile_nickname.read(cx).value().trim().to_owned();
        let signature = self.profile_signature.read(cx).value().trim().to_owned();
        let current = &bootstrap.profile;
        let request = iamrust_protocol::UpdateProfileRequest {
            nickname: if nickname.is_empty() {
                current.nickname.clone()
            } else {
                nickname
            },
            signature: if signature.is_empty() {
                current.signature.clone()
            } else {
                signature
            },
            avatar_url: current.avatar_url.as_ref().map(ToString::to_string),
            avatar_attachment_id: current.avatar_attachment_id,
            gender: current.gender.clone(),
            birthday: current.birthday,
            region: current.region.clone(),
            presence: Some(current.presence),
        };
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { api.update_profile(request) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(profile) => {
                        this.profile_name = profile_name(&profile);
                        if let Some(bootstrap) = &mut this.bootstrap {
                            bootstrap.profile = profile;
                        }
                        this.action_error = Some("资料已保存".to_owned());
                    }
                    Err(error) => this.action_error = Some(error.user_message()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_cache_encryption(&mut self, cx: &mut Context<Self>) {
        if self.action_busy {
            return;
        }
        let Some(store) = self.store.clone() else {
            self.action_error = Some("本地缓存不可用".to_owned());
            cx.notify();
            return;
        };
        let enabled = self.cache_encryption != Some(true);
        let runtime = self.runtime.clone();
        self.action_busy = true;
        let task = cx
            .background_executor()
            .spawn(async move { runtime.block_on(store.set_encryption_enabled(enabled)) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.action_busy = false;
                match result {
                    Ok(enabled) => this.cache_encryption = Some(enabled),
                    Err(error) => this.action_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_local_cache(&mut self, cx: &mut Context<Self>) {
        if !self.clear_cache_confirmation {
            self.clear_cache_confirmation = true;
            cx.notify();
            return;
        }
        self.clear_cache_confirmation = false;
        let Some(store) = self.store.clone() else {
            return;
        };
        let runtime = self.runtime.clone();
        let task = cx.background_executor().spawn(async move {
            runtime.block_on(async {
                store.clear_account_cache().await?;
                store.cache_stats().await
            })
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(stats) => {
                        this.cache_stats = Some(stats);
                        this.action_error = Some("本地缓存已清理".to_owned());
                    }
                    Err(error) => this.action_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_shell(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let content = match self.navigation {
            Navigation::Chats => h_flex()
                .h_full()
                .flex_1()
                .child(self.render_conversation_list(cx))
                .child(self.render_chat(window, cx))
                .when(self.group_details_open, |view| {
                    view.child(self.render_conversation_details(cx))
                })
                .into_any_element(),
            Navigation::Contacts => h_flex()
                .h_full()
                .flex_1()
                .child(self.render_contact_list(cx))
                .child(self.render_contact_detail(cx))
                .into_any_element(),
            Navigation::Search => self.render_global_search(cx),
            Navigation::Settings => self.render_settings(window, cx),
        };
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_navigation(cx))
            .child(content)
            .into_any_element()
    }
}

fn profile_name(profile: &iamrust_domain::UserProfile) -> String {
    if profile.nickname.trim().is_empty() {
        profile.username.clone()
    } else {
        profile.nickname.clone()
    }
}

const fn role_label(role: MemberRole) -> &'static str {
    match role {
        MemberRole::Member => "成员",
        MemberRole::Administrator => "管理员",
        MemberRole::Owner => "群主",
    }
}

fn user_display_name(bootstrap: Option<&BootstrapResponse>, user_id: UserId) -> String {
    if let Some(bootstrap) = bootstrap {
        if bootstrap.profile.id == user_id {
            return format!("{}（我）", profile_name(&bootstrap.profile));
        }
        if let Some(profile) = bootstrap
            .friends
            .iter()
            .chain(&bootstrap.friend_request_profiles)
            .find(|profile| profile.id == user_id)
        {
            return profile_name(profile);
        }
    }
    let id = user_id.to_string();
    format!("用户 {}", &id[..8])
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn format_file_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        let tenths = bytes.saturating_mul(10) / (1024 * 1024);
        format!("{}.{:01} MiB", tenths / 10, tenths % 10)
    } else if bytes >= 1024 {
        let tenths = bytes.saturating_mul(10) / 1024;
        format!("{}.{:01} KiB", tenths / 10, tenths % 10)
    } else {
        format!("{bytes} B")
    }
}

fn safe_suggested_file_name(attachment: &iamrust_domain::Attachment) -> String {
    let name = attachment.file_name.trim();
    if name.is_empty()
        || name.chars().count() > 240
        || name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        format!("attachment-{}", attachment.id)
    } else {
        name.to_owned()
    }
}

fn default_download_directory() -> PathBuf {
    let profile = if cfg!(target_os = "windows") {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    };
    if let Some(directory) = profile
        .map(PathBuf::from)
        .map(|path| path.join("Downloads"))
        && directory.is_dir()
    {
        return directory;
    }
    std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
}

fn cached_or_failed(
    runtime: &Runtime,
    store: Option<&LocalStore>,
    remote_error: String,
) -> RestoreOutcome {
    let Some(store) = store else {
        return RestoreOutcome::Failed(remote_error);
    };
    match runtime.block_on(store.load_bootstrap()) {
        Ok(Some(bootstrap)) => RestoreOutcome::Cached(bootstrap, remote_error),
        Ok(None) => RestoreOutcome::Failed(remote_error),
        Err(cache_error) => RestoreOutcome::Failed(format!("{remote_error}；{cache_error}")),
    }
}

fn load_timeline(
    api: Option<&ApiClient>,
    store: Option<&LocalStore>,
    runtime: &Runtime,
    conversation_id: iamrust_domain::ConversationId,
) -> TimelineOutcome {
    let cached = store.map_or_else(
        || Ok(Vec::new()),
        |store| runtime.block_on(store.load_messages(&conversation_id.to_string())),
    );
    let Some(api) = api else {
        return match cached {
            Ok(messages) if !messages.is_empty() => {
                TimelineOutcome::Cached(messages, "服务器地址配置无效".to_owned())
            }
            Ok(_) => TimelineOutcome::Failed("服务器地址配置无效".to_owned()),
            Err(error) => TimelineOutcome::Failed(error),
        };
    };
    match api.messages(conversation_id, None, 50) {
        Ok(page) => {
            let next_cursor = page_next_cursor(&page.items, page.next_cursor.as_deref(), None);
            let cache_warning =
                store.and_then(|store| runtime.block_on(store.cache_messages(&page.items)).err());
            let mut messages = page.items;
            if let Ok(cached_messages) = &cached {
                let mut known = messages
                    .iter()
                    .map(|message| message.client_message_id)
                    .collect::<HashSet<_>>();
                messages.extend(
                    cached_messages
                        .iter()
                        .filter(|message| {
                            matches!(
                                message.status,
                                MessageStatus::Pending | MessageStatus::Failed
                            ) && known.insert(message.client_message_id)
                        })
                        .cloned(),
                );
            }
            TimelineOutcome::Live(messages, next_cursor, cache_warning)
        }
        Err(error) => match cached {
            Ok(messages) if !messages.is_empty() => {
                TimelineOutcome::Cached(messages, error.user_message())
            }
            Ok(_) => TimelineOutcome::Failed(error.user_message()),
            Err(cache_error) => {
                TimelineOutcome::Failed(format!("{}；{cache_error}", error.user_message()))
            }
        },
    }
}

fn page_next_cursor(
    items: &[Message],
    raw_cursor: Option<&str>,
    before: Option<u64>,
) -> Option<u64> {
    if items.len() < 50 {
        return None;
    }
    raw_cursor
        .and_then(|cursor| cursor.parse::<u64>().ok())
        .filter(|cursor| before.is_none_or(|before| *cursor < before))
}

fn flush_ready_outbox(
    api: Arc<ApiClient>,
    store: &LocalStore,
    runtime: &Runtime,
) -> OutboxFlushReport {
    let mut report = OutboxFlushReport {
        sent: Vec::new(),
        retrying: Vec::new(),
        cache_warning: None,
    };
    let items = match runtime.block_on(store.ready_outbox()) {
        Ok(items) => items,
        Err(error) => {
            report.cache_warning = Some(error);
            return report;
        }
    };
    for item in items {
        let parsed = (
            uuid::Uuid::parse_str(&item.client_message_id)
                .ok()
                .map(MessageId::from_uuid),
            uuid::Uuid::parse_str(&item.conversation_id)
                .ok()
                .map(ConversationId::from_uuid),
            serde_json::from_str::<SendMessageRequest>(&item.payload_json).ok(),
        );
        let (Some(client_message_id), Some(conversation_id), Some(request)) = parsed else {
            if let Err(error) = runtime.block_on(store.acknowledge_outbox(&item.client_message_id))
            {
                report.cache_warning = Some(error);
            }
            continue;
        };
        if request.client_message_id != client_message_id {
            if let Err(error) = runtime.block_on(store.acknowledge_outbox(&item.client_message_id))
            {
                report.cache_warning = Some(error);
            }
            continue;
        }
        match api.send_message(conversation_id, &request) {
            Ok(ack) => {
                match runtime.block_on(store.load_messages(&item.conversation_id)) {
                    Ok(mut messages) => {
                        if let Some(message) = messages
                            .iter_mut()
                            .find(|message| message.client_message_id == client_message_id)
                        {
                            message.id = ack.message_id;
                            if message.mark_sent(ack.sequence, ack.server_time).is_ok()
                                && let Err(error) = runtime
                                    .block_on(store.cache_messages(std::slice::from_ref(message)))
                            {
                                report.cache_warning = Some(error);
                            }
                        }
                    }
                    Err(error) => report.cache_warning = Some(error),
                }
                if let Err(error) =
                    runtime.block_on(store.acknowledge_outbox(&item.client_message_id))
                {
                    report.cache_warning = Some(error);
                }
                report.sent.push((client_message_id, ack.message_id));
            }
            Err(error) => {
                if let Err(cache_error) = runtime
                    .block_on(store.record_outbox_failure(&item.client_message_id, "send_failed"))
                {
                    report.cache_warning = Some(cache_error);
                }
                report
                    .retrying
                    .push((client_message_id, error.user_message()));
            }
        }
    }

    report
}

fn upload_attachment_and_send(
    api: Arc<ApiClient>,
    store: Option<&LocalStore>,
    runtime: &Runtime,
    send: AttachmentSendContext,
) -> AttachmentUploadOutcome {
    let completed = match api.upload_file(&send.upload.path, send.upload.image) {
        Ok(completed) => completed,
        Err(error) => {
            return AttachmentUploadOutcome::Failed {
                client_message_id: send.client_message_id,
                error: error.user_message(),
            };
        }
    };
    let attachment = completed.attachment;
    let label = if send.upload.image {
        format!("[图片] {}", attachment.file_name)
    } else {
        format!("[文件] {}", attachment.file_name)
    };
    let content = if send.upload.image {
        MessageContent::Image {
            attachment: attachment.clone(),
        }
    } else {
        MessageContent::File {
            attachment: attachment.clone(),
        }
    };
    let mut pending = match Message::pending(
        send.client_message_id,
        send.conversation_id,
        send.sender_id,
        content,
        Utc::now(),
    ) {
        Ok(pending) => pending,
        Err(error) => {
            return AttachmentUploadOutcome::Failed {
                client_message_id: send.client_message_id,
                error: error.to_string(),
            };
        }
    };
    pending.reply_to = send.reply_to;
    AttachmentUploadOutcome::Completed {
        label,
        attachment,
        send: send_pending_message(api, store, runtime, pending),
    }
}

fn send_pending_message(
    api: Arc<ApiClient>,
    store: Option<&LocalStore>,
    runtime: &Runtime,
    mut pending: Message,
) -> SendOutcome {
    let request = SendMessageRequest {
        client_message_id: pending.client_message_id,
        content: pending.content.clone(),
        reply_to: pending.reply_to,
        mentions: Vec::new(),
        mention_all: false,
        expires_in_seconds: None,
    };
    let mut cache_warning = None;
    if let Some(store) = store {
        if let Err(error) = runtime.block_on(store.cache_messages(std::slice::from_ref(&pending))) {
            cache_warning = Some(error);
        }
        match serde_json::to_string(&request) {
            Ok(payload) => {
                if let Err(error) = runtime.block_on(store.enqueue_outbox(
                    &pending.client_message_id.to_string(),
                    &pending.conversation_id.to_string(),
                    &payload,
                )) {
                    cache_warning = Some(error);
                }
            }
            Err(_) => cache_warning = Some("发送队列序列化失败".to_owned()),
        }
    }

    match api.send_message(pending.conversation_id, &request) {
        Ok(ack) => {
            pending.id = ack.message_id;
            if pending.mark_sent(ack.sequence, ack.server_time).is_err() {
                return SendOutcome::Failed {
                    client_message_id: pending.client_message_id,
                    error: "消息状态更新失败".to_owned(),
                    cache_warning,
                };
            }
            if let Some(store) = store {
                if let Err(error) = runtime.block_on(store.cache_messages(&[pending.clone()])) {
                    cache_warning = Some(error);
                }
                if let Err(error) = runtime
                    .block_on(store.acknowledge_outbox(&pending.client_message_id.to_string()))
                {
                    cache_warning = Some(error);
                }
            }
            SendOutcome::Sent {
                client_message_id: pending.client_message_id,
                message_id: pending.id,
                cache_warning,
            }
        }
        Err(error) => {
            let _ = pending.mark_failed();
            if let Some(store) = store {
                if let Err(cache_error) = runtime.block_on(store.cache_messages(&[pending.clone()]))
                {
                    cache_warning = Some(cache_error);
                }
                if let Err(cache_error) =
                    runtime.block_on(store.record_outbox_failure(
                        &pending.client_message_id.to_string(),
                        "send_failed",
                    ))
                {
                    cache_warning = Some(cache_error);
                }
            }
            SendOutcome::Failed {
                client_message_id: pending.client_message_id,
                error: error.user_message(),
                cache_warning,
            }
        }
    }
}

fn notification_text(
    conversation: &str,
    author: &str,
    body: &str,
    privacy_mode: bool,
) -> (String, String) {
    if privacy_mode {
        return ("I Am Rust".to_owned(), "你收到了一条新消息".to_owned());
    }

    let title = sanitize_notification_text(if conversation.trim().is_empty() {
        "新消息"
    } else {
        conversation.trim()
    });
    let author = sanitize_notification_text(if author.trim().is_empty() {
        "联系人"
    } else {
        author.trim()
    });
    let body = sanitize_notification_text(body.trim());
    (
        elide_text(&title, 60),
        elide_text(&format!("{author}：{body}"), 180),
    )
}

fn sanitize_notification_text(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '<' => '‹',
            '>' => '›',
            '\n' | '\t' => character,
            character if character.is_control() => ' ',
            character => character,
        })
        .collect()
}

fn elide_text(text: &str, max_characters: usize) -> String {
    if text.chars().count() <= max_characters {
        return text.to_owned();
    }
    let mut elided = text
        .chars()
        .take(max_characters.saturating_sub(1))
        .collect::<String>();
    elided.push('…');
    elided
}

impl Render for IamRustApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(if self.authenticated {
            self.render_shell(window, cx)
        } else {
            self.render_auth(cx)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_cursor_requires_a_full_strictly_older_page() {
        let conversation_id = ConversationId::new();
        let sender_id = UserId::new();
        let mut messages = (0..50)
            .map(|index| {
                Message::pending(
                    MessageId::new(),
                    conversation_id,
                    sender_id,
                    MessageContent::Text {
                        text: format!("message {index}"),
                    },
                    Utc::now(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();

        assert_eq!(page_next_cursor(&messages, Some("100"), None), Some(100));
        assert_eq!(page_next_cursor(&messages, Some("99"), Some(100)), Some(99));
        assert_eq!(page_next_cursor(&messages, Some("100"), Some(100)), None);
        assert_eq!(page_next_cursor(&messages, Some("invalid"), None), None);
        messages.pop();
        assert_eq!(page_next_cursor(&messages, Some("98"), None), None);
    }

    #[test]
    fn private_notifications_do_not_leak_message_metadata() {
        assert_eq!(
            notification_text("Alice", "Alice", "secret", true),
            ("I Am Rust".to_owned(), "你收到了一条新消息".to_owned())
        );
    }

    #[test]
    fn notification_text_is_safe_and_unicode_aware() {
        let (title, body) = notification_text("<Rust 群>", "Ferris", "<b>你好</b>", false);
        assert_eq!(title, "‹Rust 群›");
        assert_eq!(body, "Ferris：‹b›你好‹/b›");
        assert_eq!(elide_text("铁铁铁铁", 3), "铁铁…");
    }
}
