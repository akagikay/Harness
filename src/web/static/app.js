// Harness Client - Agentic Workspace Interface (Zero External Frameworks)

let activeEventSource = null;
let activeEventStreamSessionId = null;
let currentSessionId = null;
let activeReasoningDrawer = null;
let activeMessageBox = null;
let activeTurnId = null;
let activeSendMode = 'queue';
let queuedPrompts = [];
let isSubmittingPrompt = false;
let currentConfig = null;
let activeExecutionMode = 'Autonomous';
let mockModeEnabled = false;
let isMainSessionRunning = false;
let activeSubagentsCount = 0;
let userClosedInspector = false;
let shouldAutoScroll = true;
let isProgrammaticScroll = false;
let lastTouchY = null;
let activeDisplayVerbosity = 'compact';
let selectedSlashCommand = null;
let stagedAttachments = [];
let lastCurrentContextTokens = 0;

// Remote and cached models state
let allRemoteModels = [];
let allProviderModelGroups = [];
let activeCtxMinFilter = 0;
const activeCapFilters = new Set();
let submenuCloseTimer = null;
let submenuHoverModelId = null;
let activeModelPickerContext = null;
const modelReasoningCache = new Map();
const modelContextCache = new Map();
const modelModalitiesCache = new Map();

// Projects, Sessions, Servers, Skills, and Inspector State
const CONVERSATIONS_PROJECT_ID = '__conversations__';
let allProjects = [];
let allSessions = [];
let activeProjectId = 'project';
let activeComposerScope = 'project'; // 'project' | 'conversation'
let allServers = [];
const serverHealthMap = new Map();
let pendingMoveConvId = null;
let draftOrchState = {
    enabled: false,
    provider_id: '',
    model: '',
    thinking_effort: 'high',
    subagent_mode: 'single',
    subagent_provider_id: '',
    subagent_model: '',
    subagent_thinking_effort: 'step_down',
    subagent_server_id: 'local',
    subagent_profiles: []
};
let allSkills = [];

const toolCallsMap = new Map();
const sessionSubagents = new Map();
let swarmBoardMessages = [];
let swarmBoardSeqSet = new Set();
let sessionSwarmJobs = [];
let sessionSwarmSpecialists = [];
let swarmTitle = '';
let swarmCoordinator = null;
let activeSwarmSubtab = 'board';
let swarmBoardFilterMode = 'all';
let swarmRetiredCollapsed = false;
let swarmStateSyncTimer = null;
let sessionArtifacts = [];
let browserSnapshots = [];
let selectedSnapshotIndex = -1;
let currentViewerContent = '';
let currentViewerIsMarkdown = false;
let currentViewerPreviewMd = false;

// DOM Elements
const streamScrollArea = document.getElementById('stream-scroll-area');
const streamContainer = document.getElementById('stream-container');
const btnScrollBottom = document.getElementById('btn-scroll-bottom');
const welcomeCanvas = document.getElementById('welcome-canvas');
const taskInput = document.getElementById('task-input');
const btnSubmit = document.getElementById('btn-submit');
const currentTaskBreadcrumb = document.getElementById('current-task-breadcrumb');
const btnRenameSession = document.getElementById('btn-rename-session');
const queuedPromptsTray = document.getElementById('queued-prompts-tray');
const settingsSendMode = document.getElementById('settings-send-mode');
const settingsDisplayVerbosity = document.getElementById('settings-display-verbosity');
const composerSlashRow = document.getElementById('composer-slash-row');
const composerSlashChip = document.getElementById('composer-slash-chip');
const composerSlashIcon = document.getElementById('composer-slash-icon');
const composerSlashCmd = document.getElementById('composer-slash-cmd');
const composerSlashLabel = document.getElementById('composer-slash-label');
const btnClearSlashChip = document.getElementById('btn-clear-slash-chip');
const promptCard = document.getElementById('prompt-card');
const composerAttachmentsRow = document.getElementById('composer-attachments-row');
const promptFileInput = document.getElementById('prompt-file-input');
const attachMenuPopover = document.getElementById('attach-menu-popover');
const btnAttachLocalFiles = document.getElementById('btn-attach-local-files');
const btnAttachWorkspaceFiles = document.getElementById('btn-attach-workspace-files');
const btnMic = document.getElementById('btn-mic');

// Telemetry & Badges
const telemetryPill = document.getElementById('telemetry-pill');
const statCpu = document.getElementById('stat-cpu');
const statRss = document.getElementById('stat-rss');
const settingsStatRss = document.getElementById('settings-stat-rss');
const statHostRam = document.getElementById('stat-host-ram');
const labelPermMode = document.getElementById('label-perm-mode');
const btnPermModeDropdown = document.getElementById('btn-perm-mode-dropdown');
const labelActiveModel = document.getElementById('label-active-model');
const labelReasoningEffort = document.getElementById('label-reasoning-effort');

// Sidebar & Navigation
const sidebar = document.getElementById('sidebar');
const btnSidebarToggle = document.getElementById('btn-sidebar-toggle');
const btnExpandSidebar = document.getElementById('btn-expand-sidebar');
const btnMobileSidebar = document.getElementById('btn-mobile-sidebar');
const btnNewChat = document.getElementById('btn-new-chat');
const btnOpenSettings = document.getElementById('btn-open-settings');
const btnOpenToolsNav = document.getElementById('btn-open-tools-nav');
const btnMoreOptions = document.getElementById('btn-more-options');
const btnPromptAttach = document.getElementById('btn-prompt-attach');

// Inspector Elements
const inspectorPane = document.getElementById('inspector-pane');
const btnToggleDrawer = document.getElementById('btn-toggle-drawer');
const btnCloseInspector = document.getElementById('btn-close-inspector');
const inspectorActivityDot = document.getElementById('inspector-activity-dot');
const inspectorSwarmBadge = document.getElementById('inspector-swarm-badge');
const inspectorBrowserBadge = document.getElementById('inspector-browser-badge');
const inspectorFilesBadge = document.getElementById('inspector-files-badge');
const inspectorAgentsSummary = document.getElementById('inspector-agents-summary');
const inspectorSwarmTagsSection = document.getElementById('inspector-swarm-tags-section');
const inspectorSwarmJobs = document.getElementById('inspector-swarm-jobs');
const inspectorSwarmSpecialists = document.getElementById('inspector-swarm-specialists');
const inspectorSubagentsList = document.getElementById('inspector-subagents-list');
const inspectorBoardMessages = document.getElementById('inspector-board-messages');
const btnRefreshSwarm = document.getElementById('btn-refresh-swarm');
const btnSwarmQuickPost = document.getElementById('btn-swarm-quick-post');
const btnSwarmExpand = document.getElementById('btn-swarm-expand');
const swarmTitleLabel = document.getElementById('swarm-title-label');
const swarmLiveIndicator = document.getElementById('swarm-live-indicator');
const swarmBoardCount = document.getElementById('swarm-board-count');
const swarmMembersActiveCount = document.getElementById('swarm-members-active-count');
const swarmBoardSearch = document.getElementById('swarm-board-search');
const btnSwarmBoardFilter = document.getElementById('btn-swarm-board-filter');
const swarmBoardFilterLabel = document.getElementById('swarm-board-filter-label');
const swarmMembersSearch = document.getElementById('swarm-members-search');
const swarmMembersActive = document.getElementById('swarm-members-active');
const swarmMembersRetiredSection = document.getElementById('swarm-members-retired-section');
const btnToggleRetired = document.getElementById('btn-toggle-retired');
const swarmRetiredCount = document.getElementById('swarm-retired-count');
const swarmMembersRetired = document.getElementById('swarm-members-retired');
const swarmGraphCanvas = document.getElementById('swarm-graph-canvas');
const swarmPostTarget = document.getElementById('swarm-post-target');
const swarmPostInput = document.getElementById('swarm-post-input');
const btnSwarmPostSend = document.getElementById('btn-swarm-post-send');

const browserTestUrl = document.getElementById('browser-test-url');
const btnBrowserAudit = document.getElementById('btn-browser-audit');
const btnBrowserIframeToggle = document.getElementById('btn-browser-iframe-toggle');
const browserIframeWrap = document.getElementById('browser-iframe-wrap');
const browserLiveIframe = document.getElementById('browser-live-iframe');
const browserConsoleBadge = document.getElementById('browser-console-badge');
const browserSnapshotViewer = document.getElementById('browser-snapshot-viewer');
const browserSnapshotGallery = document.getElementById('browser-snapshot-gallery');

const inspectorArtifactsCount = document.getElementById('inspector-artifacts-count');
const inspectorArtifactsList = document.getElementById('inspector-artifacts-list');
const workspaceRootLabel = document.getElementById('workspace-root-label');
const btnWorkspaceRefresh = document.getElementById('btn-workspace-refresh');
const workspaceTreeList = document.getElementById('workspace-tree-list');
const workspaceFileViewer = document.getElementById('workspace-file-viewer');
const fileViewerPath = document.getElementById('file-viewer-path');
const fileViewerLang = document.getElementById('file-viewer-lang');
const fileViewerBody = document.getElementById('file-viewer-body');
const btnToggleMdPreview = document.getElementById('btn-toggle-md-preview');
const btnCopyViewerFile = document.getElementById('btn-copy-viewer-file');
const btnCloseFileViewer = document.getElementById('btn-close-file-viewer');

// Image Lightbox Elements
const imageLightboxModal = document.getElementById('image-lightbox-modal');
const lightboxTitle = document.getElementById('lightbox-title');
const lightboxImg = document.getElementById('lightbox-img');
const btnCloseLightbox = document.getElementById('btn-close-lightbox');

// Popovers
const modelPopover = document.getElementById('model-popover');
const btnModelDropdown = document.getElementById('btn-model-dropdown');
const modelSearchInput = document.getElementById('model-search-input');
const btnClearModelSearch = document.getElementById('btn-clear-model-search');
const btnToggleModelFilters = document.getElementById('btn-toggle-model-filters');
const modelFiltersDrawer = document.getElementById('model-filters-drawer');
const modelCtxFilterRow = document.getElementById('model-ctx-filter-row');
const modelCapFilterRow = document.getElementById('model-cap-filter-row');
const modelHoverDetail = document.getElementById('model-hover-detail');
const btnQuickFetchModels = document.getElementById('btn-quick-fetch-models');
const btnOpenAddProviderPill = document.getElementById('btn-open-add-provider-pill');
const quickModelsList = document.getElementById('quick-models-list');
const labelActiveProviderName = document.getElementById('label-active-provider-name');
const labelModelCtxSummary = document.getElementById('label-model-ctx-summary');
const btnOpenProvidersModal = document.getElementById('btn-open-providers-modal');

const thinkingEffortSubmenu = document.getElementById('thinking-effort-submenu');
const thinkingEffortList = document.getElementById('thinking-effort-list');
const thinkingSubmenuModelHint = document.getElementById('thinking-submenu-model-hint');
const permModePopover = document.getElementById('perm-mode-popover');
const morePopover = document.getElementById('more-popover');

// Separated Sidebar Conversations & Server / Orchestrator UI Elements
const sidebarConversationsList = document.getElementById('sidebar-conversations-list');
const btnAddConversation = document.getElementById('btn-add-conversation');
const btnServerDropdown = document.getElementById('btn-server-dropdown');
const serverStatusDot = document.getElementById('server-status-dot');
const labelActiveServer = document.getElementById('label-active-server');
const serverPopover = document.getElementById('server-popover');
const serverPopoverList = document.getElementById('server-popover-list');
const btnManageServersFromPop = document.getElementById('btn-manage-servers-from-pop');
const moveConvPopover = document.getElementById('move-conv-popover');
const moveConvList = document.getElementById('move-conv-list');

const orchEnabled = document.getElementById('orch-enabled');
const btnOrchModelPicker = document.getElementById('btn-orch-model-picker');
const labelOrchModel = document.getElementById('label-orch-model');
const labelOrchEffort = document.getElementById('label-orch-effort');
const labelOrchProviderHint = document.getElementById('label-orch-provider-hint');
const orchSubagentModeCtrl = document.getElementById('orch-subagent-mode-ctrl');
const orchSingleSubagentPanel = document.getElementById('orch-single-subagent-panel');
const orchMultiSubagentPanel = document.getElementById('orch-multi-subagent-panel');
const btnSubagentModelPicker = document.getElementById('btn-subagent-model-picker');
const labelSubagentModel = document.getElementById('label-subagent-model');
const labelSubagentEffort = document.getElementById('label-subagent-effort');
const labelSubagentProviderHint = document.getElementById('label-subagent-provider-hint');
const orchSubagentServer = document.getElementById('orch-subagent-server');
const btnAddSubagentProfile = document.getElementById('btn-add-subagent-profile');
const subagentProfilesContainer = document.getElementById('subagent-profiles-container');
const orchSystemPrompt = document.getElementById('orch-system-prompt');
const btnResetOrchPrompt = document.getElementById('btn-reset-orch-prompt');
const btnSaveOrchestrator = document.getElementById('btn-save-orchestrator');

const projectSettingsModal = document.getElementById('project-settings-modal');
const btnCloseProjSettings = document.getElementById('btn-close-proj-settings');
const btnCancelProjSettings = document.getElementById('btn-cancel-proj-settings');
const btnSaveProjSettings = document.getElementById('btn-save-proj-settings');
const projSettingsIcon = document.getElementById('proj-settings-icon');
const projSettingsTitle = document.getElementById('proj-settings-title');
const projDefName = document.getElementById('proj-def-name');
const projDefIcon = document.getElementById('proj-def-icon');
const projDefPath = document.getElementById('proj-def-path');
const btnBrowseProjDefPath = document.getElementById('btn-browse-proj-def-path');
const btnProjDefModelPicker = document.getElementById('btn-proj-def-model-picker');
const labelProjDefModel = document.getElementById('label-proj-def-model');
const labelProjDefEffort = document.getElementById('label-proj-def-effort');
const btnClearProjDefModel = document.getElementById('btn-clear-proj-def-model');
const labelProjDefProviderHint = document.getElementById('label-proj-def-provider-hint');
const projDefServer = document.getElementById('proj-def-server');
const projDefOrchestrator = document.getElementById('proj-def-orchestrator');
let draftProjSettings = null;

const btnShowAddServerForm = document.getElementById('btn-show-add-server-form');
const serverEditorCard = document.getElementById('server-editor-card');
const btnCancelServerEditor = document.getElementById('btn-cancel-server-editor');
const btnDismissServerEditor = document.getElementById('btn-dismiss-server-editor');
const btnSaveServer = document.getElementById('btn-save-server');
const configuredServersContainer = document.getElementById('configured-servers-container');
const editServerName = document.getElementById('edit-server-name');
const editServerId = document.getElementById('edit-server-id');
const editServerUrl = document.getElementById('edit-server-url');
const editServerToken = document.getElementById('edit-server-token');
const editServerDefault = document.getElementById('edit-server-default');

// Settings Hub - Providers Section
const btnShowAddProviderForm = document.getElementById('btn-show-add-provider-form');
const providerEditorCard = document.getElementById('provider-editor-card');
const btnCancelProviderEditor = document.getElementById('btn-cancel-provider-editor');
const btnDismissProviderEditor = document.getElementById('btn-dismiss-provider-editor');
const btnSaveCustomProvider = document.getElementById('btn-save-custom-provider');
const btnTestProviderConnection = document.getElementById('btn-test-provider-connection');
const providerTestStatus = document.getElementById('provider-test-status');
const configuredProvidersContainer = document.getElementById('configured-providers-container');

// Settings Hub - MCP Section & Google AI Search Spotlight
const googleMcpEnabled = document.getElementById('google-mcp-enabled');
const googleMcpApiKey = document.getElementById('google-mcp-api-key');
const googleMcpModel = document.getElementById('google-mcp-model');
const btnSaveGoogleMcp = document.getElementById('btn-save-google-mcp');
const btnTestGoogleMcp = document.getElementById('btn-test-google-mcp');
const googleMcpStatus = document.getElementById('google-mcp-status');
const btnShowAddMcpForm = document.getElementById('btn-show-add-mcp-form');
const mcpEditorCard = document.getElementById('mcp-editor-card');
const btnCancelMcpEditor = document.getElementById('btn-cancel-mcp-editor');
const btnDismissMcpEditor = document.getElementById('btn-dismiss-mcp-editor');
const btnSaveMcpServer = document.getElementById('btn-save-mcp-server');
const btnTestMcpServer = document.getElementById('btn-test-mcp-server');
const mcpTestStatus = document.getElementById('mcp-test-status');
const configuredMcpContainer = document.getElementById('configured-mcp-container');

// MCP Editor Fields
const editMcpName = document.getElementById('edit-mcp-name');
const editMcpId = document.getElementById('edit-mcp-id');
const editMcpTransport = document.getElementById('edit-mcp-transport');
const editMcpCommand = document.getElementById('edit-mcp-command');
const editMcpArgs = document.getElementById('edit-mcp-args');
const editMcpUrl = document.getElementById('edit-mcp-url');
const editMcpEnv = document.getElementById('edit-mcp-env');

// Settings Hub - Search Section
const searchSettingsTitle = document.getElementById('search-settings-title');
const cfgSearchApiKey = document.getElementById('cfg-search-api-key');
const cfgSearchMaxResults = document.getElementById('cfg-search-max-results');
const cfgSearxngUrl = document.getElementById('cfg-searxng-url');
const btnSaveSearchConfig = document.getElementById('btn-save-search-config');
const searchTestQuery = document.getElementById('search-test-query');
const btnRunSearchTest = document.getElementById('btn-run-search-test');
const searchTestOutputBox = document.getElementById('search-test-output-box');
const testOutputProviderBadge = document.getElementById('test-output-provider-badge');
const testOutputLatency = document.getElementById('test-output-latency');
const testOutputRendered = document.getElementById('test-output-rendered');

// Projects DOM Elements
const sidebarProjectsList = document.getElementById('sidebar-projects-list');
const btnNewProjectFolder = document.getElementById('btn-new-project-folder');
const btnToggleProjectsSort = document.getElementById('btn-toggle-projects-sort');
const projectModal = document.getElementById('project-modal');
const btnCloseProject = document.getElementById('btn-close-project');
const btnCancelProject = document.getElementById('btn-cancel-project');
const btnSubmitProject = document.getElementById('btn-submit-project');
const projectNameInput = document.getElementById('project-name-input');
const projectPathInput = document.getElementById('project-path-input');
const projectSetActiveCheck = document.getElementById('project-set-active-check');
const welcomeProjectTitle = document.getElementById('welcome-project-title');

// Sidebar Inline Project Creator & File Browser Elements
const sidebarInlineProjectForm = document.getElementById('sidebar-inline-project-form');
const btnCloseInlineProject = document.getElementById('btn-close-inline-project');
const btnCancelInlineProject = document.getElementById('btn-cancel-inline-project');
const btnSubmitInlineProject = document.getElementById('btn-submit-inline-project');
const inlineProjectName = document.getElementById('inline-project-name');
const inlineProjectPath = document.getElementById('inline-project-path');
const btnBrowseInlineProject = document.getElementById('btn-browse-inline-project');
const btnToggleInlineTree = document.getElementById('btn-toggle-inline-tree');
const inlineBrowserDrawer = document.getElementById('inline-browser-drawer');
const inlineBrowserShortcuts = document.getElementById('inline-browser-shortcuts');
const inlineBrowserCurrentLabel = document.getElementById('inline-browser-current-label');
const btnInlineBrowserUp = document.getElementById('btn-inline-browser-up');
const btnInlineBrowserMkdir = document.getElementById('btn-inline-browser-mkdir');
const btnInlineBrowserUse = document.getElementById('btn-inline-browser-use');
const inlineBrowserTreeList = document.getElementById('inline-browser-tree-list');

// Context Telemetry & Token Analytics Popover Elements
const btnTokenAnalytics = document.getElementById('btn-token-analytics');
const tokenAnalyticsPopover = document.getElementById('token-analytics-popover');
const btnPopoverCompactCtx = document.getElementById('btn-popover-compact-ctx');
const statCtxTokens = document.getElementById('stat-ctx-tokens');
const statCtxPct = document.getElementById('stat-ctx-pct');
const analyticsCtxUtilText = document.getElementById('analytics-ctx-util-text');
const analyticsCtxProgress = document.getElementById('analytics-ctx-progress');
const analyticsPromptTokens = document.getElementById('analytics-prompt-tokens');
const analyticsCompletionTokens = document.getElementById('analytics-completion-tokens');
const analyticsCachedTokens = document.getElementById('analytics-cached-tokens');
const analyticsCacheHitRate = document.getElementById('analytics-cache-hit-rate');
const analyticsTotalTokens = document.getElementById('analytics-total-tokens');
const analyticsCostUsd = document.getElementById('analytics-cost-usd');
const analyticsModelWindowHint = document.getElementById('analytics-model-window-hint');

// In-Browser File & Folder Browser Elements
const btnBrowseProjectPath = document.getElementById('btn-browse-project-path');
const fileBrowserModal = document.getElementById('file-browser-modal');
const btnCloseFileBrowser = document.getElementById('btn-close-file-browser');
const btnCancelBrowser = document.getElementById('btn-cancel-browser');
const btnConfirmBrowserSelect = document.getElementById('btn-confirm-browser-select');
const browserPathInput = document.getElementById('browser-path-input');
const btnBrowserBack = document.getElementById('btn-browser-back');
const btnBrowserUp = document.getElementById('btn-browser-up');
const btnBrowserRefresh = document.getElementById('btn-browser-refresh');
const btnBrowserMkdir = document.getElementById('btn-browser-mkdir');
const browserDirsOnlyToggle = document.getElementById('browser-dirs-only-toggle');
const browserFavoritesList = document.getElementById('browser-favorites-list');
const browserDrivesList = document.getElementById('browser-drives-list');
const browserEntriesList = document.getElementById('browser-entries-list');
const browserSelectedLabel = document.getElementById('browser-selected-label');

// Task Running & Stop Button Elements
const taskRunningStrip = document.getElementById('task-running-strip');
const btnTopStopSession = document.getElementById('btn-top-stop-session');
const btnStopPrompt = document.getElementById('btn-stop-prompt');
const taskRunningLabel = document.getElementById('task-running-label');

// Command Palette DOM Elements
const commandPaletteModal = document.getElementById('command-palette-modal');
const paletteSearchInput = document.getElementById('palette-search-input');
const paletteResultsList = document.getElementById('palette-results-list');
const btnSearchSessions = document.getElementById('btn-search-sessions');

// Prompt Autocomplete DOM Elements
const promptAutocompleteMenu = document.getElementById('prompt-autocomplete-menu');
const autocompleteHeader = document.getElementById('autocomplete-header');
const autocompleteList = document.getElementById('autocomplete-list');

// Provider Editor Fields
const editProviderName = document.getElementById('edit-provider-name');
const editProviderId = document.getElementById('edit-provider-id');
const editProviderBase = document.getElementById('edit-provider-base');
const editProviderKey = document.getElementById('edit-provider-key');
const editProviderModel = document.getElementById('edit-provider-model');

const configModal = document.getElementById('config-modal');
const btnCloseConfig = document.getElementById('btn-close-config');
const btnCancelConfig = document.getElementById('btn-cancel-config');
const btnSaveConfig = document.getElementById('btn-save-config');
const btnFetchModels = document.getElementById('btn-fetch-models');
const settingsSaveFeedback = document.getElementById('settings-save-feedback');
const btnCompactActiveSession = document.getElementById('btn-compact-active-session');
const settingsCompactStatus = document.getElementById('settings-compact-status');
const btnOpenToolsFromSettings = document.getElementById('btn-open-tools-from-settings');
const settingsToolsCountBadge = document.getElementById('settings-tools-count-badge');

// Settings Hub - Skills & Permissions Section
const btnSavePermissions = document.getElementById('btn-save-permissions');
const btnNewSkill = document.getElementById('btn-new-skill');
const btnRefreshSkills = document.getElementById('btn-refresh-skills');
const skillEditorCard = document.getElementById('skill-editor-card');
const skillEditorTitle = document.getElementById('skill-editor-title');
const editSkillName = document.getElementById('edit-skill-name');
const editSkillDesc = document.getElementById('edit-skill-desc');
const editSkillContent = document.getElementById('edit-skill-content');
const editSkillEnabled = document.getElementById('edit-skill-enabled');
const btnSaveSkill = document.getElementById('btn-save-skill');
const btnCancelSkill = document.getElementById('btn-cancel-skill');
const btnDismissSkillEditor = document.getElementById('btn-dismiss-skill-editor');
const skillsListContainer = document.getElementById('skills-list-container');

const toolsModal = document.getElementById('tools-modal');
const btnCloseTools = document.getElementById('btn-close-tools');
const btnCloseToolsFoot = document.getElementById('btn-close-tools-foot');
const toolsContainer = document.getElementById('tools-container');

// Config Form Fields (Runtime + MemoryConfig)
const cfgApiBase = document.getElementById('cfg-api-base');
const cfgApiKey = document.getElementById('cfg-api-key');
const cfgModel = document.getElementById('cfg-model');
const cfgReasoningEffort = document.getElementById('cfg-reasoning-effort');
const cfgMaxOut = document.getElementById('cfg-max-output-bytes');
const cfgSandbox = document.getElementById('cfg-sandbox');
const cfgMaxContextTokens = document.getElementById('cfg-max-context-tokens');
const cfgMaxContextAutoHint = document.getElementById('cfg-max-context-auto-hint');
const cfgAutoDetectContext = document.getElementById('cfg-auto-detect-context');
const cfgAutoCompactPct = document.getElementById('cfg-auto-compact-pct');
const cfgPreserveRecent = document.getElementById('cfg-preserve-recent');
const cfgPruneToolBytes = document.getElementById('cfg-prune-tool-bytes');
const cfgMaxToolRetries = document.getElementById('cfg-max-tool-retries');
const cfgEnableWorkingMemory = document.getElementById('cfg-enable-working-memory');
const cfgSummarizeCompactedTurns = document.getElementById('cfg-summarize-compacted-turns');
const cfgVoiceActivityDetection = document.getElementById('cfg-voice-activity-detection');
const cfgAutoUpdateCheck = document.getElementById('cfg-auto-update-check');
const cfgAutoUpdateApply = document.getElementById('cfg-auto-update-apply');
const cfgUpdatePin = document.getElementById('cfg-update-pin');
const cfgPrivilegedShell = document.getElementById('cfg-privileged-shell');

/// The privileged `bash` tool is a security boundary, not a preference, so it is
/// NOT written through the bulk config snapshot (a stale tab must not silently
/// hand the agent an unjailed shell). It round-trips through its own endpoint.
function syncPrivilegedShellUi(enabled) {
    const state = Boolean(enabled);
    if (cfgPrivilegedShell) cfgPrivilegedShell.checked = state;
    const warn = document.getElementById('privileged-shell-warning');
    if (warn) {
        warn.classList.toggle('hidden', !state);
        warn.setAttribute('aria-hidden', state ? 'false' : 'true');
    }
}

async function setPrivilegedShell(next) {
    const res = await fetch('/api/security/privileged-shell', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled: next })
    });
    if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
    return res.json();
}
const cfgAutoSkillRouter = document.getElementById('cfg-auto-skill-router');
const cfgSkillRouterConfidence = document.getElementById('cfg-skill-router-confidence');
const cfgSkillRouterMax = document.getElementById('cfg-skill-router-max');
const cfgReasoningCapabilityBadge = document.getElementById('cfg-reasoning-capability-badge');

const DEFAULT_NVIDIA_KEY = ""; // no bundled key; user supplies their own NVIDIA key
const DEFAULT_TINYFISH_KEY = ""; // no bundled key

// ==========================================================================
// Auth token support (optional). When the server sets HARNESS_AUTH_TOKEN the
// UI prompts once and attaches it to every API call.
// ==========================================================================
function harnessAuthToken() {
    return localStorage.getItem('harness_auth_token') || '';
}
let __harnessTokenPromptActive = false;
function promptForHarnessToken() {
    // De-duplicate: several API calls fire at boot, so without this the user
    // would be prompted once per in-flight request.
    if (__harnessTokenPromptActive) return false;
    __harnessTokenPromptActive = true;
    try {
        const entered = window.prompt('This Harness server requires an access token:');
        if (entered && entered.trim()) {
            localStorage.setItem('harness_auth_token', entered.trim());
            return true;
        }
        return false;
    } finally {
        __harnessTokenPromptActive = false;
    }
}
// Attach the bearer token to same-origin /api requests automatically.
const __harnessNativeFetch = window.fetch.bind(window);
function __harnessApplyToken(input, init) {
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    const isApi = url.startsWith('/api/') || url.includes('://' + location.host + '/api/');
    if (!isApi) return { isApi, init };
    const token = harnessAuthToken();
    if (!token) return { isApi, init };
    init = init || {};
    const headers = new Headers(init.headers || {});
    if (!headers.has('Authorization')) headers.set('Authorization', 'Bearer ' + token);
    init.headers = headers;
    return { isApi, init };
}
window.fetch = function (input, init) {
    const first = __harnessApplyToken(input, init);
    return __harnessNativeFetch(input, first.init).then(res => {
        if (res.status === 401 && first.isApi && promptForHarnessToken()) {
            // Rebuild the request so the freshly entered token is actually sent.
            const retry = __harnessApplyToken(input, init);
            return __harnessNativeFetch(input, retry.init);
        }
        return res;
    });
};
// EventSource cannot send headers; append ?token= for SSE endpoints.
function harnessSseUrl(url) {
    const token = harnessAuthToken();
    if (!token) return url;
    return url + (url.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(token);
}

// ==========================================================================
// Automatic updates: live progress drawer (checkbox + download + install)
// ==========================================================================
let updateStatus = { phase: 'idle' };
let updateDrawerDismissed = false;
let updateRestartOverlayShown = false;

function fmtBytes(bytes) {
    if (!bytes || bytes < 0) return '';
    const units = ['B', 'KB', 'MB', 'GB'];
    let n = bytes;
    let u = 0;
    while (n >= 1024 && u < units.length - 1) { n /= 1024; u++; }
    return `${n >= 100 || u === 0 ? Math.round(n) : n.toFixed(1)} ${units[u]}`;
}

async function fetchUpdateStatus() {
    const drawer = document.getElementById('update-drawer');
    if (!drawer) return;
    const overlay = document.getElementById('update-restart-overlay');
    let data;
    try {
        const res = await fetch('/api/update/status');
        data = await res.json();
    } catch (_) {
        return; // server mid-restart; keep the last rendered state
    }
    if (!data || typeof data !== 'object') return;
    const previous = updateStatus.phase;
    updateStatus = data;

    // Once the install lands, block the UI and reload into the new binary.
    if (data.phase === 'restarting') {
        updateRestartOverlayShown = true;
        drawer.classList.add('hidden');
        if (overlay) {
            overlay.classList.remove('hidden');
            const title = document.getElementById('update-restart-title');
            const msg = document.getElementById('update-restart-message');
            if (title) title.textContent = `Updated to v${data.target_version || ''}`;
            if (msg) msg.textContent = 'Restarting Harness. This tab will reload automatically.';
        }
        setTimeout(() => { window.location.reload(); }, 3500);
    } else if (updateRestartOverlayShown) {
        updateRestartOverlayShown = false;
        overlay?.classList.add('hidden');
    }

    if (data.phase === 'available' && previous !== 'available') {
        showToast(`Harness v${data.target_version} is available - installing automatically.`, 'info', 8000);
    }
    if (data.phase === 'failed' && previous !== 'failed') {
        const msg = data.error || 'unknown error';
        if (data.transient) {
            showToast('Update check delayed - ' + msg + ' (will retry automatically).', 'warning', 9000);
        } else {
            showToast('Update failed: ' + msg, 'error', 9000);
        }
    }
    const runningVersionEl = document.getElementById('cfg-running-version');
    if (runningVersionEl && data.current_version) {
        const label = 'v' + data.current_version;
        if (runningVersionEl.textContent !== label) runningVersionEl.textContent = label;
    }
    renderUpdateDrawer(data);
}

function renderUpdateDrawer(data) {
    const drawer = document.getElementById('update-drawer');
    if (!drawer) return;

    const quiet = data.phase === 'idle' || data.phase === 'up_to_date';
    if (quiet || (updateDrawerDismissed && data.phase !== 'failed')) {
        drawer.classList.add('hidden');
        return;
    }

    const icon = document.getElementById('update-drawer-icon');
    const title = document.getElementById('update-drawer-title');
    const barWrap = document.getElementById('update-drawer-bar');
    const fill = document.getElementById('update-drawer-bar-fill');
    const bytes = document.getElementById('update-drawer-bytes');
    const pct = document.getElementById('update-drawer-percent');
    const installBtn = document.getElementById('update-drawer-install');
    const link = document.getElementById('update-drawer-link');

    drawer.classList.remove('hidden');
    drawer.setAttribute('data-phase', data.phase);

    const total = data.total_bytes || 0;
    const done = data.downloaded_bytes || 0;
    const percent = data.percent != null
        ? data.percent
        : (total > 0 ? Math.min(100, Math.round((done / total) * 100)) : null);

    const labels = {
        checking: 'Checking for updates\u2026',
        available: `Harness v${data.target_version || ''} available`,
        downloading: `Downloading v${data.target_version || ''}\u2026`,
        verifying: 'Verifying checksum\u2026',
        waiting_for_idle: 'Waiting for running work to finish\u2026',
        installing: 'Installing update\u2026',
        restarting: 'Restarting\u2026',
        failed: 'Update failed'
    };
    if (title) {
        if (data.phase === 'failed') {
            title.textContent = data.transient
                ? 'Update check delayed \u2014 retrying soon'
                : (data.error || labels.failed);
        } else {
            title.textContent = labels[data.phase] || data.message || 'Updating\u2026';
        }
    }
    if (icon) icon.textContent = data.phase === 'failed' ? '\u26A0' : '\u21BB';

    const downloading = data.phase === 'downloading';
    if (barWrap) barWrap.classList.toggle('active', downloading);
    if (fill) fill.style.width = `${downloading ? (percent == null ? 0 : percent) : 100}%`;
    if (bytes) bytes.textContent = downloading
        ? `${fmtBytes(done)}${total ? ' / ' + fmtBytes(total) : ''}`
        : '';
    if (pct) pct.textContent = downloading && percent != null ? `${percent}%` : '';

    if (installBtn) {
        installBtn.classList.toggle('hidden', data.phase !== 'available');
        installBtn.onclick = manualUpdateInstall;
    }
    if (link) {
        if (data.html_url) {
            link.href = data.html_url;
            link.classList.remove('hidden');
        } else {
            link.classList.add('hidden');
        }
    }
}

async function manualUpdateInstall() {
    const btn = document.getElementById('update-drawer-install');
    if (btn) { btn.disabled = true; btn.textContent = 'Starting\u2026'; }
    try {
        const res = await fetch('/api/update/apply', { method: 'POST' });
        if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
        showToast('Update started.', 'success');
    } catch (e) {
        showToast('Update failed: ' + e.message, 'error', 7000);
        if (btn) { btn.disabled = false; btn.textContent = 'Install now'; }
    }
}

// Kept for callers/tests: the background loop is authoritative now.
async function checkForHarnessUpdate() {
    await fetchUpdateStatus();
}

// ==========================================================================
// Bespoke UI Controls (Toast, Confirm/Prompt Modals, Custom Select & Combobox)
// ==========================================================================
function showToast(message, type = 'info', duration = 4200) {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const toast = document.createElement('div');
    toast.className = `toast-item ${type}`;
    const icons = { info: 'ℹ', success: '✓', error: '✕', warning: '⚠' };
    toast.innerHTML = `
        <span class="toast-icon">${icons[type] || icons.info}</span>
        <span class="toast-msg">${escapeHtml(String(message || ''))}</span>
        <button type="button" class="toast-close" aria-label="Dismiss">✕</button>
    `;
    const closeBtn = toast.querySelector('.toast-close');
    const dismiss = () => {
        if (toast.parentNode) toast.remove();
    };
    if (closeBtn) closeBtn.addEventListener('click', dismiss);
    container.appendChild(toast);
    if (duration > 0) setTimeout(dismiss, duration);
}

let activeDialogResolver = null;
function openUiDialog({ title = 'Confirm', message = '', mode = 'confirm', defaultValue = '', placeholder = '', confirmText = 'Confirm', cancelText = 'Cancel', danger = false }) {
    const modal = document.getElementById('ui-dialog-modal');
    const titleEl = document.getElementById('ui-dialog-title');
    const msgEl = document.getElementById('ui-dialog-message');
    const inputWrap = document.getElementById('ui-dialog-input-wrap');
    const inputEl = document.getElementById('ui-dialog-input');
    const btnOk = document.getElementById('btn-ui-dialog-confirm');
    const btnCancel = document.getElementById('btn-ui-dialog-cancel');
    const btnClose = document.getElementById('btn-ui-dialog-close');

    if (!modal || !titleEl || !msgEl || !btnOk || !btnCancel) {
        return Promise.resolve(mode === 'prompt' ? null : false);
    }

    if (activeDialogResolver) {
        activeDialogResolver(mode === 'prompt' ? null : false);
        activeDialogResolver = null;
    }

    titleEl.textContent = title;
    msgEl.textContent = message;
    btnOk.textContent = confirmText;
    btnOk.className = `btn-primary${danger ? ' danger' : ''}`;
    btnCancel.textContent = cancelText;

    if (mode === 'prompt' && inputWrap && inputEl) {
        inputWrap.classList.remove('hidden');
        inputEl.value = defaultValue || '';
        inputEl.placeholder = placeholder || '';
    } else if (inputWrap) {
        inputWrap.classList.add('hidden');
    }

    modal.classList.remove('hidden');

    return new Promise((resolve) => {
        const finish = (result) => {
            modal.classList.add('hidden');
            document.removeEventListener('keydown', onKey);
            btnOk.onclick = null;
            btnCancel.onclick = null;
            if (btnClose) btnClose.onclick = null;
            modal.onclick = null;
            if (inputEl) inputEl.onkeydown = null;
            activeDialogResolver = null;
            resolve(result);
        };
        activeDialogResolver = finish;

        const accept = () => {
            if (mode === 'prompt') finish(inputEl ? inputEl.value : '');
            else finish(true);
        };
        const dismiss = () => finish(mode === 'prompt' ? null : false);

        const onKey = (e) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                dismiss();
            }
        };
        document.addEventListener('keydown', onKey);

        btnOk.onclick = accept;
        btnCancel.onclick = dismiss;
        if (btnClose) btnClose.onclick = dismiss;
        modal.onclick = (e) => { if (e.target === modal) dismiss(); };
        if (inputEl) {
            inputEl.onkeydown = (e) => {
                if (e.key === 'Enter') {
                    e.preventDefault();
                    accept();
                }
            };
        }
        setTimeout(() => {
            if (mode === 'prompt' && inputEl) {
                inputEl.focus();
                inputEl.select();
            } else {
                btnOk.focus();
            }
        }, 10);
    });
}

function uiConfirm(message, opts = {}) {
    return openUiDialog({
        title: opts.title || 'Confirm Action',
        message,
        mode: 'confirm',
        confirmText: opts.confirmText || 'Confirm',
        cancelText: opts.cancelText || 'Cancel',
        danger: opts.danger !== false
    });
}

function uiPrompt(message, opts = {}) {
    return openUiDialog({
        title: opts.title || 'Enter Value',
        message,
        mode: 'prompt',
        defaultValue: opts.defaultValue || '',
        placeholder: opts.placeholder || '',
        confirmText: opts.confirmText || 'Create',
        cancelText: opts.cancelText || 'Cancel',
        danger: false
    });
}

function closeAllCustomDropdowns(exceptEl = null) {
    document.querySelectorAll('.ui-select.open').forEach(sel => {
        if (exceptEl && sel === exceptEl) return;
        sel.classList.remove('open');
        const menu = sel.querySelector('.ui-select-menu');
        const trigger = sel.querySelector('.ui-select-trigger');
        if (menu) menu.classList.add('hidden');
        if (trigger) trigger.setAttribute('aria-expanded', 'false');
    });
    document.querySelectorAll('.ui-combobox-menu:not(.hidden)').forEach(menu => {
        if (exceptEl && menu.parentElement === exceptEl) return;
        menu.classList.add('hidden');
    });
}

function bindCustomSelect(sel) {
    if (!sel || sel.dataset.bound === '1') return;
    sel.dataset.bound = '1';
    const trigger = sel.querySelector('.ui-select-trigger');
    const menu = sel.querySelector('.ui-select-menu');
    const hiddenInput = sel.querySelector('input[type="hidden"]');
    const labelEl = sel.querySelector('.ui-select-label');
    if (!trigger || !menu) return;

    trigger.addEventListener('click', (e) => {
        e.stopPropagation();
        if (sel.classList.contains('disabled')) return;
        const wasOpen = sel.classList.contains('open');
        closeAllCustomDropdowns(wasOpen ? null : sel);
        if (!wasOpen) {
            sel.classList.add('open');
            menu.classList.remove('hidden');
            trigger.setAttribute('aria-expanded', 'true');
        } else {
            sel.classList.remove('open');
            menu.classList.add('hidden');
            trigger.setAttribute('aria-expanded', 'false');
        }
    });

    menu.addEventListener('click', (e) => {
        e.stopPropagation();
        const opt = e.target.closest('.ui-select-option');
        if (!opt) return;
        const val = opt.getAttribute('data-value') ?? '';
        sel.dataset.value = val;
        if (labelEl) labelEl.textContent = opt.textContent.trim();
        menu.querySelectorAll('.ui-select-option').forEach(o => o.classList.toggle('active', o === opt));
        if (hiddenInput) {
            hiddenInput.value = val;
            hiddenInput.dispatchEvent(new Event('change', { bubbles: true }));
        }
        sel.classList.remove('open');
        menu.classList.add('hidden');
        trigger.setAttribute('aria-expanded', 'false');
    });
}

function setCustomSelectValue(hiddenInputIdOrEl, value) {
    const inputEl = typeof hiddenInputIdOrEl === 'string'
        ? document.getElementById(hiddenInputIdOrEl)
        : hiddenInputIdOrEl;
    if (!inputEl) return;
    const sel = inputEl.closest('.ui-select');
    if (!sel) {
        inputEl.value = value;
        return;
    }
    const menu = sel.querySelector('.ui-select-menu');
    const labelEl = sel.querySelector('.ui-select-label');
    inputEl.value = value;
    sel.dataset.value = value;
    if (!menu) return;
    let matchedText = null;
    menu.querySelectorAll('.ui-select-option').forEach(opt => {
        const isMatch = opt.getAttribute('data-value') === value;
        opt.classList.toggle('active', isMatch);
        if (isMatch) matchedText = opt.textContent.trim();
    });
    if (labelEl && matchedText !== null) labelEl.textContent = matchedText;
}

function setCustomSelectOptions(selectWrapperId, options, selectedValue, disabledText = null) {
    const sel = document.getElementById(selectWrapperId);
    if (!sel) return;
    const menu = sel.querySelector('.ui-select-menu');
    const labelEl = sel.querySelector('.ui-select-label');
    const hiddenInput = sel.querySelector('input[type="hidden"]');
    if (!menu) return;

    menu.innerHTML = '';
    if (disabledText !== null) {
        sel.classList.add('disabled');
        if (labelEl) labelEl.textContent = disabledText;
        if (hiddenInput) hiddenInput.value = selectedValue || '';
        return;
    }

    sel.classList.remove('disabled');
    let chosen = options.find(o => o.value === selectedValue) || options[0];
    options.forEach(o => {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `ui-select-option${chosen && chosen.value === o.value ? ' active' : ''}`;
        btn.setAttribute('data-value', o.value);
        btn.textContent = o.label;
        menu.appendChild(btn);
    });
    if (chosen) {
        sel.dataset.value = chosen.value;
        if (hiddenInput) hiddenInput.value = chosen.value;
        if (labelEl) labelEl.textContent = chosen.label;
    }
}

function normalizeModelSearchText(text) {
    return String(text || '')
        .toLowerCase()
        .replace(/[-_/]+/g, ' ')
        .replace(/\s+/g, ' ')
        .trim();
}

function matchesModelSearch(query, ...fields) {
    const normQuery = normalizeModelSearchText(query);
    if (!normQuery) return true;
    const rawCombined = fields.map(f => String(f || '').toLowerCase()).join(' ');
    const normCombined = normalizeModelSearchText(rawCombined);
    const dotFreeCombined = normCombined.replace(/\./g, ' ').replace(/\s+/g, ' ');
    const tokens = normQuery.split(' ').filter(Boolean);
    return tokens.every(tok => {
        const tokNoDots = tok.replace(/\./g, ' ').trim();
        return normCombined.includes(tok)
            || rawCombined.includes(tok)
            || (tokNoDots && dotFreeCombined.includes(tokNoDots));
    });
}

function initCombobox(inputId, menuId, chevronId, getItemsFn, onSelectFn) {
    const input = document.getElementById(inputId);
    const menu = document.getElementById(menuId);
    const chevron = document.getElementById(chevronId);
    if (!input || !menu) return;

    const renderMenu = (filterStr = '') => {
        const items = getItemsFn().filter(item => {
            if (!filterStr || !filterStr.trim()) return true;
            return matchesModelSearch(filterStr, item.value, item.label, item.meta);
        });
        menu.innerHTML = '';
        if (items.length === 0) {
            menu.innerHTML = '<div class="ui-combobox-empty">No matching items</div>';
            return;
        }
        items.slice(0, 60).forEach(item => {
            const btn = document.createElement('button');
            btn.type = 'button';
            btn.className = `ui-select-option${item.value === input.value.trim() ? ' active' : ''}`;
            btn.innerHTML = `
                <span class="ui-select-label">${escapeHtml(item.label || item.value)}</span>
                ${item.meta ? `<span class="ui-combobox-item-meta">${escapeHtml(item.meta)}</span>` : ''}
            `;
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                input.value = item.value;
                menu.classList.add('hidden');
                if (onSelectFn) onSelectFn(item.value, item);
            });
            menu.appendChild(btn);
        });
    };

    input.addEventListener('focus', () => {
        closeAllCustomDropdowns(input.parentElement);
        renderMenu(input.value);
        menu.classList.remove('hidden');
    });
    input.addEventListener('input', () => {
        renderMenu(input.value);
        menu.classList.remove('hidden');
    });
    if (chevron) {
        chevron.addEventListener('click', (e) => {
            e.stopPropagation();
            const isHidden = menu.classList.contains('hidden');
            closeAllCustomDropdowns(isHidden ? input.parentElement : null);
            if (isHidden) {
                renderMenu('');
                menu.classList.remove('hidden');
                input.focus();
            } else {
                menu.classList.add('hidden');
            }
        });
    }
    menu.addEventListener('click', (e) => e.stopPropagation());
}

function formatReasoningLevelLabel(level) {
    const map = {
        minimal: 'Minimal',
        low: 'Low',
        medium: 'Medium',
        high: 'High',
        none: 'Off'
    };
    return map[level] || (level ? level.charAt(0).toUpperCase() + level.slice(1) : 'High');
}

function getDefaultReasoningBadgeMeta(spec) {
    if (!spec || spec.support === 'fixed') {
        return { cls: 'fixed', text: 'Fixed', label: 'Reasoning: Fixed' };
    }
    if (spec.support === 'none') {
        return { cls: 'unsupported', text: 'Not Exposed', label: 'No Reasoning' };
    }
    const levels = Array.isArray(spec.levels) ? spec.levels : [];
    return {
        cls: 'configurable',
        text: levels.join(' · '),
        label: null
    };
}

// ==========================================================================
// Bootstrap
// ==========================================================================
document.addEventListener('DOMContentLoaded', () => {
    seedExperimentalModelCaches();
    document.querySelectorAll('.ui-select').forEach(bindCustomSelect);

    initCombobox(
        'cfg-model',
        'cfg-model-menu',
        'btn-cfg-model-dropdown',
        () => {
            const list = allRemoteModels.length > 0 ? allRemoteModels : getFallbackModelsList();
            return list.map(m => {
                const spec = m.reasoning || getModelReasoningSpec(m.id);
                let meta = 'Standard';
                if (spec.support === 'configurable') meta = `${spec.levels.join('/')}`;
                else if (spec.support === 'fixed') meta = 'Fixed';
                return { value: m.id, label: m.id, meta };
            });
        },
        (selectedId) => {
            refreshReasoningControlsForModel(selectedId, false);
            syncModelContextWindowIfAuto(selectedId);
        }
    );

    initCombobox(
        'ct-category',
        'ct-category-menu',
        'btn-ct-category-dropdown',
        () => [
            { value: 'custom', label: 'custom' },
            { value: 'shell', label: 'shell' },
            { value: 'web', label: 'web' },
            { value: 'files', label: 'files' },
            { value: 'devops', label: 'devops' }
        ],
        null
    );

    if (cfgModel) {
        cfgModel.addEventListener('change', () => {
            const val = cfgModel.value.trim();
            if (val) {
                refreshReasoningControlsForModel(val, false);
                syncModelContextWindowIfAuto(val);
            }
        });
    }
    if (cfgAutoDetectContext) {
        cfgAutoDetectContext.addEventListener('change', () => {
            syncAutoContextUiState();
            if (cfgAutoDetectContext.checked) {
                const modelId = (cfgModel && cfgModel.value.trim()) || (currentConfig && currentConfig.model) || '';
                if (modelId) syncModelContextWindowIfAuto(modelId);
            }
        });
    }
    if (cfgReasoningEffort) {
        cfgReasoningEffort.addEventListener('change', () => {
            const overrideModel = (cfgModel && cfgModel.value.trim()) || null;
            setReasoningEffort(cfgReasoningEffort.value, false, true, overrideModel);
        });
    }

    fetchMetrics();
    setInterval(fetchMetrics, 2500);
    // Live auto-update progress (download %, verify, install, restart).
    fetchUpdateStatus();
    setInterval(fetchUpdateStatus, 1200);
    const hideUpdateDrawer = document.getElementById('update-drawer-close');
    if (hideUpdateDrawer) {
        hideUpdateDrawer.addEventListener('click', () => {
            updateDrawerDismissed = true;
            document.getElementById('update-drawer')?.classList.add('hidden');
        });
    }
    fetchProjects();
    fetchSessions();
    fetchSkills();
    loadConfig();
    // Non-blocking: surface a newer GitHub release if one exists.
    checkForHarnessUpdate();

    // Auto-grow textarea & Prompt Autocomplete
    taskInput.addEventListener('input', () => {
        autoResizeTextarea();
        checkPromptAutocomplete();
    });
    taskInput.addEventListener('keydown', handleTaskInputKeydown);

    if (btnClearSlashChip) {
        btnClearSlashChip.addEventListener('click', (e) => {
            e.stopPropagation();
            clearComposerSlashCommand();
            if (taskInput) taskInput.focus();
        });
    }

    // Streaming Auto-Scroll Lock & Jump-to-Bottom Pill
    if (streamScrollArea) {
        streamScrollArea.addEventListener('scroll', () => {
            if (isProgrammaticScroll) return;
            const dist = getDistanceFromBottom();
            if (dist > 60) {
                syncShouldAutoScrollFlag(false);
            } else if (dist <= 24) {
                syncShouldAutoScrollFlag(true);
            } else {
                updateScrollBottomButton();
            }
        }, { passive: true });

        streamScrollArea.addEventListener('wheel', (e) => {
            if (e.deltaY < 0) syncShouldAutoScrollFlag(false);
        }, { passive: true });

        streamScrollArea.addEventListener('touchstart', (e) => {
            if (e.touches && e.touches[0]) lastTouchY = e.touches[0].clientY;
        }, { passive: true });

        streamScrollArea.addEventListener('touchmove', (e) => {
            if (e.touches && e.touches[0] && lastTouchY !== null && e.touches[0].clientY > lastTouchY + 4) {
                syncShouldAutoScrollFlag(false);
            } else if (getDistanceFromBottom() > 60) {
                syncShouldAutoScrollFlag(false);
            }
            if (e.touches && e.touches[0]) lastTouchY = e.touches[0].clientY;
        }, { passive: true });
    }
    if (streamContainer) {
        streamContainer.shouldAutoScroll = shouldAutoScroll;
        streamContainer.addEventListener('wheel', (e) => {
            if (e.deltaY < 0) syncShouldAutoScrollFlag(false);
        }, { passive: true });
        streamContainer.addEventListener('touchmove', () => {
            if (getDistanceFromBottom() > 60) syncShouldAutoScrollFlag(false);
        }, { passive: true });
    }
    if (btnScrollBottom) {
        btnScrollBottom.addEventListener('click', () => {
            scrollToBottom(true);
        });
    }

    // Clicking the active task / swarm pill opens the Swarm Inspector tab
    if (taskRunningStrip) {
        taskRunningStrip.addEventListener('click', (e) => {
            if (e.target && e.target.closest('#btn-top-stop-session')) return;
            userClosedInspector = false;
            openInspector('swarm');
        });
    }

    // Display Verbosity Lever (Settings Modal)
    if (settingsDisplayVerbosity) {
        settingsDisplayVerbosity.querySelectorAll('[data-verbosity]').forEach(btn => {
            btn.addEventListener('click', () => {
                setDisplayVerbosity(btn.getAttribute('data-verbosity') || 'compact', true);
            });
        });
    }
    setDisplayVerbosity(activeDisplayVerbosity, false);

    btnSubmit.addEventListener('click', handlePromptSubmit);
    // The sidebar CTA and Ctrl+N must do the same thing: a PROJECT chat (with file
    // & shell tools). The sidebar label advertises ^N, so they cannot differ.
    btnNewChat.addEventListener('click', () => startNewConversation(false));

    // Steer / Queue Segmented Control (Settings Modal)
    if (settingsSendMode) {
        settingsSendMode.querySelectorAll('[data-send-mode]').forEach(btn => {
            btn.addEventListener('click', () => {
                setSendMode(btn.getAttribute('data-send-mode') || 'queue', true);
            });
        });
    }
    setSendMode(activeSendMode, false);

    // Inline Session Rename (Double-click breadcrumb or click rename icon)
    if (currentTaskBreadcrumb) {
        currentTaskBreadcrumb.addEventListener('dblclick', () => {
            if (currentSessionId) startInlineRenameSession();
        });
    }
    if (btnRenameSession) {
        btnRenameSession.addEventListener('click', (e) => {
            e.stopPropagation();
            if (currentSessionId) startInlineRenameSession();
        });
    }

    // Sidebar Toggles
    // Below 64em the sidebar is an off-canvas drawer instead of a collapsing
    // column, so the "collapse" button must close (not hide) it there.
    const sidebarIsDrawer = () => window.matchMedia('(max-width: 64em)').matches;
    const closeSidebarDrawer = () => {
        sidebar.classList.remove('open');
    };
    const toggleSidebar = () => {
        if (sidebarIsDrawer()) {
            const wasOpen = sidebar.classList.contains('open');
            if (wasOpen) closeSidebarDrawer();
            else sidebar.classList.add('open');
            return;
        }
        const collapsed = sidebar.classList.toggle('collapsed');
        document.body.classList.toggle('sidebar-is-collapsed', collapsed);
    };
    if (btnSidebarToggle) btnSidebarToggle.addEventListener('click', toggleSidebar);
    if (btnExpandSidebar) btnExpandSidebar.addEventListener('click', toggleSidebar);
    if (btnMobileSidebar) btnMobileSidebar.addEventListener('click', toggleSidebar);
    // Tapping the workspace (or pressing Escape) dismisses the drawer.
    document.addEventListener('click', (e) => {
        if (!sidebar.classList.contains('open')) return;
        if (sidebar.contains(e.target)) return;
        if (e.target.closest && e.target.closest('#btn-mobile-sidebar, #btn-sidebar-toggle, .top-nav-left')) return;
        closeSidebarDrawer();
    });
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && sidebar.classList.contains('open')) closeSidebarDrawer();
    });
    // Leaving drawer mode (window widened) must not strand the sidebar off-screen.
    window.matchMedia('(min-width: 64.0625em)').addEventListener?.('change', (ev) => {
        if (ev.matches) closeSidebarDrawer();
    });

    // Inspector Toggles & Tabs
    if (btnToggleDrawer) {
        btnToggleDrawer.addEventListener('click', () => {
            const isCollapsed = inspectorPane.classList.contains('collapsed');
            if (isCollapsed) {
                userClosedInspector = false;
                openInspector();
            } else {
                userClosedInspector = true;
                closeInspector();
            }
        });
    }
    if (btnCloseInspector) {
        btnCloseInspector.addEventListener('click', () => {
            userClosedInspector = true;
            closeInspector();
        });
    }

    document.querySelectorAll('.inspector-tab').forEach(tab => {
        tab.addEventListener('click', () => {
            const target = tab.getAttribute('data-inspector-tab');
            switchInspectorTab(target);
        });
    });

    // Swarm Board Composer & Subtabs
    document.querySelectorAll('.swarm-subtab').forEach(btn => {
        btn.addEventListener('click', () => {
            const sub = btn.getAttribute('data-swarm-subtab');
            if (sub) switchSwarmSubtab(sub);
        });
    });
    if (btnSwarmQuickPost) {
        btnSwarmQuickPost.addEventListener('click', () => {
            switchSwarmSubtab('board');
            if (swarmPostInput) swarmPostInput.focus();
        });
    }
    if (btnSwarmExpand && inspectorPane) {
        btnSwarmExpand.addEventListener('click', () => {
            const expanded = inspectorPane.classList.toggle('swarm-expanded');
            btnSwarmExpand.classList.toggle('active', expanded);
            if (activeSwarmSubtab === 'graph') renderSwarmGraph();
        });
    }
    if (swarmBoardSearch) {
        swarmBoardSearch.addEventListener('input', () => renderInspectorBoard());
    }
    if (btnSwarmBoardFilter) {
        btnSwarmBoardFilter.addEventListener('click', () => {
            const order = ['all', 'mentions', 'tagged'];
            const idx = order.indexOf(swarmBoardFilterMode);
            swarmBoardFilterMode = order[(idx + 1) % order.length];
            btnSwarmBoardFilter.classList.toggle('active', swarmBoardFilterMode !== 'all');
            if (swarmBoardFilterLabel) {
                if (swarmBoardFilterMode === 'all') {
                    swarmBoardFilterLabel.classList.add('hidden');
                    swarmBoardFilterLabel.textContent = '';
                } else {
                    swarmBoardFilterLabel.classList.remove('hidden');
                    swarmBoardFilterLabel.textContent = swarmBoardFilterMode === 'mentions' ? '@mentions' : 'tagged';
                }
            }
            renderInspectorBoard();
        });
    }
    if (swarmMembersSearch) {
        swarmMembersSearch.addEventListener('input', () => renderInspectorSwarm());
    }
    if (btnToggleRetired && swarmMembersRetiredSection) {
        btnToggleRetired.addEventListener('click', () => {
            swarmRetiredCollapsed = !swarmRetiredCollapsed;
            swarmMembersRetiredSection.classList.toggle('collapsed', swarmRetiredCollapsed);
            btnToggleRetired.setAttribute('aria-expanded', swarmRetiredCollapsed ? 'false' : 'true');
        });
    }
    if (btnRefreshSwarm) {
        btnRefreshSwarm.addEventListener('click', () => {
            if (currentSessionId) fetchSessionSwarmState(currentSessionId);
        });
    }
    if (btnSwarmPostSend) btnSwarmPostSend.addEventListener('click', postSwarmMessage);
    if (swarmPostInput) {
        swarmPostInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') {
                e.preventDefault();
                postSwarmMessage();
            }
        });
    }

    // Browser Inspector Controls
    if (btnBrowserAudit) {
        btnBrowserAudit.addEventListener('click', () => {
            const url = (browserTestUrl ? browserTestUrl.value.trim() : '') || 'http://localhost:3000';
            taskInput.value = `Use browser_ui to navigate to ${url}, capture a screenshot, inspect the DOM, and report any console errors.`;
            autoResizeTextarea();
            handlePromptSubmit();
        });
    }
    if (btnBrowserIframeToggle) {
        btnBrowserIframeToggle.addEventListener('click', () => {
            if (!browserIframeWrap || !browserLiveIframe) return;
            const willOpen = browserIframeWrap.classList.contains('hidden');
            browserIframeWrap.classList.toggle('hidden', !willOpen);
            btnBrowserIframeToggle.textContent = willOpen ? 'Hide Frame' : 'Live Frame';
            if (willOpen) {
                const url = (browserTestUrl ? browserTestUrl.value.trim() : '') || 'http://localhost:3000';
                browserLiveIframe.src = url;
            }
        });
    }

    // Workspace Tree & File Viewer Controls
    if (btnWorkspaceRefresh) btnWorkspaceRefresh.addEventListener('click', loadWorkspaceTree);
    if (btnCloseFileViewer) {
        btnCloseFileViewer.addEventListener('click', () => {
            if (workspaceFileViewer) workspaceFileViewer.classList.add('hidden');
        });
    }
    if (btnCopyViewerFile) {
        btnCopyViewerFile.addEventListener('click', () => {
            if (!currentViewerContent) return;
            navigator.clipboard.writeText(currentViewerContent).then(() => {
                const orig = btnCopyViewerFile.textContent;
                btnCopyViewerFile.textContent = '✓ Copied';
                setTimeout(() => { btnCopyViewerFile.textContent = orig; }, 1400);
            }).catch(() => {});
        });
    }
    if (btnToggleMdPreview) {
        btnToggleMdPreview.addEventListener('click', () => {
            currentViewerPreviewMd = !currentViewerPreviewMd;
            renderFileViewerContent();
        });
    }

    // Image Lightbox Controls
    if (btnCloseLightbox) {
        btnCloseLightbox.addEventListener('click', () => imageLightboxModal.classList.add('hidden'));
    }
    if (imageLightboxModal) {
        imageLightboxModal.addEventListener('click', (e) => {
            if (e.target === imageLightboxModal) imageLightboxModal.classList.add('hidden');
        });
    }

    // Prompt Attach Menu, File Picker, Drag & Drop, and Clipboard Image Paste
    if (btnPromptAttach) {
        btnPromptAttach.addEventListener('click', (e) => {
            e.stopPropagation();
            if (attachMenuPopover) {
                const wasHidden = attachMenuPopover.classList.contains('hidden');
                closeAllPopovers();
                if (wasHidden) {
                    const rect = btnPromptAttach.getBoundingClientRect();
                    attachMenuPopover.style.left = `${rect.left}px`;
                    attachMenuPopover.style.bottom = `${window.innerHeight - rect.top + 8}px`;
                    attachMenuPopover.classList.remove('hidden');
                }
            } else if (promptFileInput) {
                promptFileInput.click();
            }
        });
    }
    if (btnAttachLocalFiles) {
        btnAttachLocalFiles.addEventListener('click', () => {
            closeAllPopovers();
            if (promptFileInput) promptFileInput.click();
        });
    }
    if (btnAttachWorkspaceFiles) {
        btnAttachWorkspaceFiles.addEventListener('click', () => {
            closeAllPopovers();
            openInspector('workspace');
            loadWorkspaceTree();
        });
    }
    if (promptFileInput) {
        promptFileInput.addEventListener('change', async () => {
            if (promptFileInput.files && promptFileInput.files.length > 0) {
                await stageFiles(promptFileInput.files);
                promptFileInput.value = '';
            }
        });
    }
    if (taskInput) {
        taskInput.addEventListener('paste', async (e) => {
            const items = e.clipboardData && e.clipboardData.files ? e.clipboardData.files : null;
            if (items && items.length > 0) {
                e.preventDefault();
                await stageFiles(items);
            }
        });
    }
    if (promptCard) {
        promptCard.addEventListener('dragover', (e) => {
            if (e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files')) {
                e.preventDefault();
                promptCard.classList.add('drag-over');
            }
        });
        promptCard.addEventListener('dragleave', (e) => {
            if (!promptCard.contains(e.relatedTarget)) {
                promptCard.classList.remove('drag-over');
            }
        });
        promptCard.addEventListener('drop', async (e) => {
            promptCard.classList.remove('drag-over');
            const files = e.dataTransfer && e.dataTransfer.files ? e.dataTransfer.files : null;
            if (files && files.length > 0) {
                e.preventDefault();
                await stageFiles(files);
            }
        });
    }

    // Starter Prompt Chips
    document.querySelectorAll('.starter-chip').forEach(chip => {
        chip.addEventListener('click', () => {
            const prompt = chip.getAttribute('data-prompt');
            taskInput.value = prompt;
            autoResizeTextarea();
            handlePromptSubmit();
        });
    });

    // Popover Trigger: Model Picker (with Cascading Thinking Effort Submenu)
    if (btnModelDropdown && modelPopover) {
        btnModelDropdown.addEventListener('click', (e) => {
            e.stopPropagation();
            const wasHidden = modelPopover.classList.contains('hidden');
            const wasCustom = activeModelPickerContext !== null;
            closeAllPopovers();
            if (wasHidden || wasCustom) {
                openUnifiedModelPicker(btnModelDropdown, null);
            }
        });
    }

    if (modelSearchInput) {
        modelSearchInput.addEventListener('input', () => {
            const q = modelSearchInput.value.trim();
            if (btnClearModelSearch) btnClearModelSearch.classList.toggle('hidden', !q);
            hideThinkingEffortSubmenu();
            renderFilteredModels();
        });
        modelSearchInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') {
                const firstCard = quickModelsList ? quickModelsList.querySelector('.model-entry-card') : null;
                if (firstCard) {
                    e.preventDefault();
                    firstCard.click();
                }
            }
        });
    }

    if (btnClearModelSearch) {
        btnClearModelSearch.addEventListener('click', () => {
            if (modelSearchInput) modelSearchInput.value = '';
            btnClearModelSearch.classList.add('hidden');
            hideThinkingEffortSubmenu();
            renderFilteredModels();
            if (modelSearchInput) modelSearchInput.focus();
        });
    }

    if (btnQuickFetchModels) {
        btnQuickFetchModels.addEventListener('click', (e) => {
            e.stopPropagation();
            fetchRemoteModels();
        });
    }
    if (btnToggleModelFilters && modelFiltersDrawer) {
        btnToggleModelFilters.addEventListener('click', (e) => {
            e.stopPropagation();
            const willOpen = modelFiltersDrawer.classList.contains('hidden');
            modelFiltersDrawer.classList.toggle('hidden', !willOpen);
            btnToggleModelFilters.classList.toggle('active', willOpen);
        });
    }
    if (modelCtxFilterRow) {
        modelCtxFilterRow.querySelectorAll('[data-ctx-min]').forEach(btn => {
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                activeCtxMinFilter = parseInt(btn.getAttribute('data-ctx-min') || '0', 10) || 0;
                modelCtxFilterRow.querySelectorAll('[data-ctx-min]').forEach(b => b.classList.toggle('active', b === btn));
                renderFilteredModels();
            });
        });
    }
    if (modelCapFilterRow) {
        modelCapFilterRow.querySelectorAll('[data-cap]').forEach(btn => {
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                const cap = btn.getAttribute('data-cap');
                if (!cap) return;
                if (activeCapFilters.has(cap)) activeCapFilters.delete(cap);
                else activeCapFilters.add(cap);
                btn.classList.toggle('active', activeCapFilters.has(cap));
                renderFilteredModels();
            });
        });
    }
    if (btnAddConversation) {
        btnAddConversation.addEventListener('click', (e) => {
            e.stopPropagation();
            createSavedConversation(CONVERSATIONS_PROJECT_ID);
        });
    }
    if (btnOrchModelPicker) {
        btnOrchModelPicker.addEventListener('click', (e) => {
            e.stopPropagation();
            openUnifiedModelPicker(btnOrchModelPicker, {
                title: 'Orchestrator Model',
                hideOrchestrator: true,
                selectedModel: draftOrchState.model || (currentConfig && currentConfig.model) || '',
                selectedProviderId: draftOrchState.provider_id || (currentConfig && currentConfig.provider) || '',
                selectedEffort: draftOrchState.thinking_effort || 'high',
                onSelect: (sel) => {
                    draftOrchState.model = sel.model;
                    draftOrchState.provider_id = sel.provider_id;
                    if (sel.thinking_effort) draftOrchState.thinking_effort = sel.thinking_effort;
                    updateOrchPickerLabels();
                }
            });
        });
    }
    if (btnSubagentModelPicker) {
        btnSubagentModelPicker.addEventListener('click', (e) => {
            e.stopPropagation();
            openUnifiedModelPicker(btnSubagentModelPicker, {
                title: 'Subagent Model',
                hideOrchestrator: true,
                selectedModel: draftOrchState.subagent_model || draftOrchState.model || (currentConfig && currentConfig.model) || '',
                selectedProviderId: draftOrchState.subagent_provider_id || draftOrchState.provider_id || '',
                selectedEffort: draftOrchState.subagent_thinking_effort || 'medium',
                onSelect: (sel) => {
                    draftOrchState.subagent_model = sel.model;
                    draftOrchState.subagent_provider_id = sel.provider_id;
                    if (sel.thinking_effort) draftOrchState.subagent_thinking_effort = sel.thinking_effort;
                    updateOrchPickerLabels();
                }
            });
        });
    }
    if (orchSubagentModeCtrl) {
        orchSubagentModeCtrl.querySelectorAll('[data-subagent-mode]').forEach(btn => {
            btn.addEventListener('click', () => {
                draftOrchState.subagent_mode = btn.getAttribute('data-subagent-mode') || 'single';
                syncSubagentStrategyVisibility();
            });
        });
    }
    if (btnAddSubagentProfile) {
        btnAddSubagentProfile.addEventListener('click', addSubagentProfile);
    }
    if (btnCloseProjSettings) btnCloseProjSettings.addEventListener('click', closeProjectSettingsModal);
    if (btnCancelProjSettings) btnCancelProjSettings.addEventListener('click', closeProjectSettingsModal);
    if (btnSaveProjSettings) btnSaveProjSettings.addEventListener('click', saveProjectDefaults);
    if (btnBrowseProjDefPath) {
        btnBrowseProjDefPath.addEventListener('click', (e) => {
            e.stopPropagation();
            openFileBrowser(projDefPath ? projDefPath.value : '', {
                title: 'Select Project Workspace Directory',
                onSelect: (chosen) => {
                    if (projDefPath) projDefPath.value = chosen;
                }
            });
        });
    }
    if (btnProjDefModelPicker) {
        btnProjDefModelPicker.addEventListener('click', (e) => {
            e.stopPropagation();
            if (!draftProjSettings) return;
            openUnifiedModelPicker(btnProjDefModelPicker, {
                title: 'Project Default Model',
                hideOrchestrator: true,
                selectedModel: draftProjSettings.defaults.model || (currentConfig && currentConfig.model) || '',
                selectedProviderId: draftProjSettings.defaults.provider_id || (currentConfig && currentConfig.provider) || '',
                selectedEffort: draftProjSettings.defaults.reasoning_effort || (currentConfig && currentConfig.reasoning_effort) || 'high',
                onSelect: (sel) => {
                    draftProjSettings.defaults.model = sel.model;
                    draftProjSettings.defaults.provider_id = sel.provider_id;
                    draftProjSettings.defaults.reasoning_effort = sel.thinking_effort || '';
                    updateProjDefPickerLabels();
                }
            });
        });
    }
    if (btnClearProjDefModel) {
        btnClearProjDefModel.addEventListener('click', () => {
            if (!draftProjSettings) return;
            draftProjSettings.defaults.model = '';
            draftProjSettings.defaults.provider_id = '';
            draftProjSettings.defaults.reasoning_effort = '';
            updateProjDefPickerLabels();
        });
    }
    if (btnServerDropdown && serverPopover) {
        btnServerDropdown.addEventListener('click', (e) => {
            e.stopPropagation();
            const wasHidden = serverPopover.classList.contains('hidden');
            closeAllPopovers();
            if (wasHidden) {
                renderServerPopover();
                probeAllServers();
                const rect = btnServerDropdown.getBoundingClientRect();
                serverPopover.style.left = `${Math.max(8, Math.min(rect.left, window.innerWidth - 310))}px`;
                serverPopover.style.top = `${rect.bottom + 8}px`;
                serverPopover.classList.remove('hidden');
            }
        });
    }
    if (btnManageServersFromPop) {
        btnManageServersFromPop.addEventListener('click', () => {
            closeAllPopovers();
            openSettingsModal('servers');
        });
    }
    if (btnSaveOrchestrator) btnSaveOrchestrator.addEventListener('click', saveOrchestratorConfig);
    if (btnResetOrchPrompt) btnResetOrchPrompt.addEventListener('click', resetOrchestratorPrompt);
    if (btnShowAddServerForm) btnShowAddServerForm.addEventListener('click', showAddServerForm);
    if (btnCancelServerEditor) btnCancelServerEditor.addEventListener('click', hideAddServerForm);
    if (btnDismissServerEditor) btnDismissServerEditor.addEventListener('click', hideAddServerForm);
    if (btnSaveServer) btnSaveServer.addEventListener('click', saveServerNode);
    if (btnFetchModels) btnFetchModels.addEventListener('click', fetchRemoteModels);

    // Providers Manager (inside Unified Settings Hub)
    const openProvidersAction = () => {
        closeAllPopovers();
        openSettingsModal('providers');
    };
    if (btnOpenAddProviderPill) {
        btnOpenAddProviderPill.addEventListener('click', (e) => {
            e.stopPropagation();
            openProvidersAction();
            showAddProviderForm();
        });
    }
    if (btnOpenProvidersModal) btnOpenProvidersModal.addEventListener('click', openProvidersAction);

    // Submenu hover stability bridge
    if (thinkingEffortSubmenu) {
        thinkingEffortSubmenu.addEventListener('mouseenter', () => {
            if (submenuCloseTimer) {
                clearTimeout(submenuCloseTimer);
                submenuCloseTimer = null;
            }
        });
        thinkingEffortSubmenu.addEventListener('mouseleave', () => {
            scheduleCloseThinkingSubmenu();
        });
    }
    if (modelPopover) {
        modelPopover.addEventListener('mouseleave', () => {
            scheduleCloseThinkingSubmenu();
        });
    }

    // Voice Input & Settings VAD toggle
    if (btnMic) {
        btnMic.addEventListener('click', (e) => {
            e.stopPropagation();
            toggleVoiceInput();
        });
    }
    if (cfgVoiceActivityDetection) {
        cfgVoiceActivityDetection.addEventListener('change', () => {
            if (currentConfig) {
                currentConfig.voice_activity_detection = !!cfgVoiceActivityDetection.checked;
                persistDisplayVerbosityConfig();
            }
            if (voiceInputActive) {
                if (isVoiceActivityEnabled()) startVadMonitor();
                else stopVadMonitor();
            }
        });
    }
    // Automatic updates: persist the toggles straight away.
    if (cfgAutoUpdateCheck) {
        cfgAutoUpdateCheck.addEventListener('change', () => {
            if (currentConfig) {
                currentConfig.auto_update_check = !!cfgAutoUpdateCheck.checked;
                persistDisplayVerbosityConfig();
                showToast(
                    cfgAutoUpdateCheck.checked
                        ? 'Automatic update checks enabled.'
                        : 'Automatic update checks disabled.',
                    'info'
                );
            }
        });
    }
    if (cfgAutoSkillRouter) {
        cfgAutoSkillRouter.addEventListener('change', () => {
            if (currentConfig) {
                currentConfig.auto_skill_router = !!cfgAutoSkillRouter.checked;
                persistDisplayVerbosityConfig();
                showToast(
                    currentConfig.auto_skill_router
                        ? 'Automatic skill routing enabled - matching skills are injected automatically.'
                        : 'Automatic skill routing disabled - use /skill-name to invoke a skill.',
                    'info'
                );
            }
        });
    }
    if (cfgSkillRouterConfidence) {
        cfgSkillRouterConfidence.addEventListener('change', () => {
            if (currentConfig) {
                const v = parseFloat(cfgSkillRouterConfidence.value);
                currentConfig.skill_router_min_confidence = Number.isFinite(v)
                    ? Math.min(0.95, Math.max(0.05, v))
                    : 0.35;
                persistDisplayVerbosityConfig();
            }
        });
    }
    if (cfgSkillRouterMax) {
        cfgSkillRouterMax.addEventListener('change', () => {
            if (currentConfig) {
                const v = parseInt(cfgSkillRouterMax.value, 10);
                currentConfig.skill_router_max_skills = Number.isFinite(v)
                    ? Math.min(4, Math.max(0, v))
                    : 2;
                persistDisplayVerbosityConfig();
            }
        });
    }
    if (cfgAutoUpdateApply) {
        cfgAutoUpdateApply.addEventListener('change', () => {
            if (currentConfig) {
                currentConfig.auto_update_apply = !!cfgAutoUpdateApply.checked;
                persistDisplayVerbosityConfig();
                showToast(
                    cfgAutoUpdateApply.checked
                        ? 'Harness will install new releases automatically.'
                        : 'Harness will only notify about new releases.',
                    'info'
                );
            }
        });
    }
    if (cfgUpdatePin) {
        cfgUpdatePin.addEventListener('change', async () => {
            if (!currentConfig) return;
            const raw = (cfgUpdatePin.value || '').trim().replace(/^v/i, '');
            cfgUpdatePin.disabled = true;
            try {
                // The pin is authoritative (an old one downgrades, an equal one
                // suppresses updates), so it is server-owned and `POST /api/config`
                // ignores it. Saving it through the whole snapshot reported success
                // while the server dropped the value.
                const res = await fetch('/api/security/update-pin', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ pin: raw })
                });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                const saved = await res.json();
                const pin = saved.update_pin_version || '';
                currentConfig.update_pin_version = pin;
                cfgUpdatePin.value = pin;
                showToast(
                    pin
                        ? `Updates pinned to v${pin} (older versions install too).`
                        : 'Update pin cleared - following the latest release.',
                    pin ? 'info' : 'success'
                );
            } catch (e) {
                // Restore the server's value: the field must never claim a pin the
                // server refused.
                cfgUpdatePin.value = currentConfig.update_pin_version || '';
                showToast('Could not save the version pin: ' + e.message, 'error');
            } finally {
                cfgUpdatePin.disabled = false;
            }
        });
    }
    if (cfgPrivilegedShell) {
        cfgPrivilegedShell.addEventListener('change', async () => {
            const next = cfgPrivilegedShell.checked;
            cfgPrivilegedShell.disabled = true;
            try {
                const out = await setPrivilegedShell(next);
                syncPrivilegedShellUi(out.allow_privileged_shell === true);
                showToast(
                    next
                        ? 'The privileged bash tool is ENABLED. It runs with full machine privileges and is not sandboxed.'
                        : 'The privileged bash tool is disabled. File tools stay inside the workspace.',
                    next ? 'warning' : 'success'
                );
            } catch (e) {
                // A refused toggle must not leave the switch lying about the state.
                syncPrivilegedShellUi(!next);
                showToast('Could not change the bash tool: ' + e.message, 'error');
            } finally {
                cfgPrivilegedShell.disabled = false;
            }
        });
    }
    const btnCheckUpdateNow = document.getElementById('btn-check-update-now');
    if (btnCheckUpdateNow) {
        btnCheckUpdateNow.addEventListener('click', async () => {
            btnCheckUpdateNow.disabled = true;
            btnCheckUpdateNow.textContent = 'Checking...';
            try {
                const res = await fetch('/api/update/check');
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                const info = await res.json();
                const runningEl = document.getElementById('cfg-running-version');
                if (runningEl) runningEl.textContent = 'v' + (info.current_version || '?');
                showToast(
                    info.available
                        ? `Harness v${info.latest_version} is available.`
                        : `Harness is up to date (v${info.current_version}).`,
                    info.available ? 'info' : 'success'
                );
            } catch (e) {
                showToast('Update check failed: ' + e.message, 'error', 7000);
            } finally {
                btnCheckUpdateNow.disabled = false;
                btnCheckUpdateNow.textContent = 'Check for updates';
            }
        });
    }

    // Popover Trigger: Permission Mode (opens upward from chat bar)
    if (btnPermModeDropdown && permModePopover) {
        btnPermModeDropdown.addEventListener('click', (e) => {
            e.stopPropagation();
            const wasHidden = permModePopover.classList.contains('hidden');
            closeAllPopovers();
            if (wasHidden) {
                const rect = btnPermModeDropdown.getBoundingClientRect();
                permModePopover.style.top = '';
                permModePopover.style.right = '';
                permModePopover.classList.remove('hidden');
                const popWidth = permModePopover.offsetWidth || 320;
                const maxLeft = Math.max(8, window.innerWidth - popWidth - 8);
                permModePopover.style.left = `${Math.max(8, Math.min(rect.left, maxLeft))}px`;
                permModePopover.style.bottom = `${window.innerHeight - rect.top + 8}px`;
            }
        });
    }

    // Popover Trigger: More Options
    if (btnMoreOptions && morePopover) {
        btnMoreOptions.addEventListener('click', (e) => {
            e.stopPropagation();
            const wasHidden = morePopover.classList.contains('hidden');
            closeAllPopovers();
            if (wasHidden) {
                const rect = btnMoreOptions.getBoundingClientRect();
                morePopover.style.right = `${Math.max(8, window.innerWidth - rect.right)}px`;
                morePopover.style.top = `${rect.bottom + 8}px`;
                morePopover.classList.remove('hidden');
            }
        });
    }

    document.addEventListener('click', closeAllPopovers);

    [modelPopover, thinkingEffortSubmenu, permModePopover, morePopover, serverPopover, moveConvPopover, tokenAnalyticsPopover, sidebarInlineProjectForm].forEach(el => {
        if (el) el.addEventListener('click', (e) => e.stopPropagation());
    });
    document.querySelectorAll('.modal-card').forEach(card => {
        card.addEventListener('click', (e) => e.stopPropagation());
    });

    // Reasoning popover items are rendered dynamically per active model capability

    // Permission Mode Selection
    document.querySelectorAll('#perm-mode-popover .popover-item').forEach(item => {
        item.addEventListener('click', () => setPermissionMode(item.getAttribute('data-perm-mode')));
    });

    // Provider Manager Actions
    if (btnShowAddProviderForm) btnShowAddProviderForm.addEventListener('click', showAddProviderForm);
    if (btnCancelProviderEditor) btnCancelProviderEditor.addEventListener('click', hideAddProviderForm);
    if (btnDismissProviderEditor) btnDismissProviderEditor.addEventListener('click', hideAddProviderForm);
    if (btnSaveCustomProvider) btnSaveCustomProvider.addEventListener('click', saveCustomProvider);
    if (btnTestProviderConnection) btnTestProviderConnection.addEventListener('click', testProviderConnection);

    // MCP & Google AI Search Actions
    if (btnSaveGoogleMcp) btnSaveGoogleMcp.addEventListener('click', saveGoogleMcpConfig);
    if (btnTestGoogleMcp) btnTestGoogleMcp.addEventListener('click', testGoogleMcpServer);
    if (btnShowAddMcpForm) btnShowAddMcpForm.addEventListener('click', showAddMcpForm);
    if (btnCancelMcpEditor) btnCancelMcpEditor.addEventListener('click', hideAddMcpForm);
    if (btnDismissMcpEditor) btnDismissMcpEditor.addEventListener('click', hideAddMcpForm);
    if (btnSaveMcpServer) btnSaveMcpServer.addEventListener('click', saveMcpServer);
    if (btnTestMcpServer) btnTestMcpServer.addEventListener('click', testMcpServerForm);

    const pMem = document.getElementById('preset-mcp-memory');
    if (pMem) pMem.addEventListener('click', () => applyMcpPreset('memory'));
    const pFs = document.getElementById('preset-mcp-fs');
    if (pFs) pFs.addEventListener('click', () => applyMcpPreset('fs'));
    const pSql = document.getElementById('preset-mcp-sqlite');
    if (pSql) pSql.addEventListener('click', () => applyMcpPreset('sqlite'));
    const pGit = document.getElementById('preset-mcp-git');
    if (pGit) pGit.addEventListener('click', () => applyMcpPreset('git'));
    const pFetch = document.getElementById('preset-mcp-fetch');
    if (pFetch) pFetch.addEventListener('click', () => applyMcpPreset('fetch'));
    // Provider template chips carry `data-preset`; the delegated click handler
    // at the bottom of this file wires them (no inline handler attributes, so
    // the page ships with a `script-src 'self'` CSP).

    if (editMcpTransport) {
        editMcpTransport.addEventListener('change', () => {
            const isSse = editMcpTransport.value === 'sse';
            const cmdWrap = document.getElementById('mcp-command-wrap');
            const argsWrap = document.getElementById('mcp-args-wrap');
            const urlWrap = document.getElementById('mcp-url-wrap');
            if (cmdWrap) cmdWrap.classList.toggle('hidden', isSse);
            if (argsWrap) argsWrap.classList.toggle('hidden', isSse);
            if (urlWrap) urlWrap.classList.toggle('hidden', !isSse);
        });
    }

    // Search Settings Actions
    if (btnSaveSearchConfig) btnSaveSearchConfig.addEventListener('click', saveSearchConfig);
    if (btnRunSearchTest) btnRunSearchTest.addEventListener('click', runSearchTest);
    if (searchTestQuery) {
        searchTestQuery.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') {
                e.preventDefault();
                runSearchTest();
            }
        });
    }

    document.querySelectorAll('.search-provider-card').forEach(card => {
        card.addEventListener('click', () => selectSearchProvider(card.getAttribute('data-search-provider')));
    });

    // Unified Tabbed Settings Hub
    if (btnOpenSettings) btnOpenSettings.addEventListener('click', () => openSettingsModal('memory'));
    if (btnCloseConfig) btnCloseConfig.addEventListener('click', () => configModal.classList.add('hidden'));
    if (btnCancelConfig) btnCancelConfig.addEventListener('click', () => configModal.classList.add('hidden'));
    if (btnSaveConfig) btnSaveConfig.addEventListener('click', saveConfig);
    if (btnCompactActiveSession) btnCompactActiveSession.addEventListener('click', compactCurrentSession);
    if (btnOpenToolsFromSettings) {
        btnOpenToolsFromSettings.addEventListener('click', () => {
            if (configModal) configModal.classList.add('hidden');
            openToolsModal();
        });
    }

    document.querySelectorAll('.settings-nav-item[data-settings-tab]').forEach(btn => {
        btn.addEventListener('click', () => switchSettingsTab(btn.getAttribute('data-settings-tab')));
    });

    if (btnSavePermissions) btnSavePermissions.addEventListener('click', savePermissions);
    if (btnNewSkill) btnNewSkill.addEventListener('click', () => showSkillEditor());
    if (btnRefreshSkills) btnRefreshSkills.addEventListener('click', fetchSkills);
    if (btnCancelSkill) btnCancelSkill.addEventListener('click', hideSkillEditor);
    if (btnDismissSkillEditor) btnDismissSkillEditor.addEventListener('click', hideSkillEditor);
    if (btnSaveSkill) btnSaveSkill.addEventListener('click', saveSkill);

    if (btnOpenToolsNav) btnOpenToolsNav.addEventListener('click', openToolsModal);
    if (btnCloseTools) btnCloseTools.addEventListener('click', () => toolsModal.classList.add('hidden'));
    if (btnCloseToolsFoot) btnCloseToolsFoot.addEventListener('click', () => toolsModal.classList.add('hidden'));
    initToolsModalControls();

    // Segmented controls
    document.querySelectorAll('.segmented-control').forEach(ctrl => {
        ctrl.querySelectorAll('.seg-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                ctrl.querySelectorAll('.seg-btn').forEach(b => b.classList.remove('active'));
                btn.classList.add('active');
            });
        });
    });

    const permGlobalMode = document.getElementById('perm-global-mode');
    if (permGlobalMode) {
        permGlobalMode.querySelectorAll('.seg-btn').forEach(btn => {
            btn.addEventListener('click', () => setPermissionMode(btn.getAttribute('data-val')));
        });
    }

    // Provider Presets in Settings
    const presetBtns = {
        'preset-nvidia': 'nvidia',
        'preset-deepseek': 'deepseek',
        'preset-openrouter': 'openrouter',
        'preset-groq': 'groq',
        'preset-ollama': 'ollama',
    };
    Object.entries(presetBtns).forEach(([btnId, pId]) => {
        const btn = document.getElementById(btnId);
        if (btn) btn.addEventListener('click', () => switchProvider(pId));
    });

    // More options dropdown actions
    const optForkSession = document.getElementById('opt-fork-session');
    if (optForkSession) {
        optForkSession.addEventListener('click', () => {
            closeAllPopovers();
            forkCurrentSession(null);
        });
    }
    const optRenameSession = document.getElementById('opt-rename-session');
    if (optRenameSession) {
        optRenameSession.addEventListener('click', () => {
            closeAllPopovers();
            if (currentSessionId) startInlineRenameSession();
            else showToast('No active conversation to rename.', 'warning');
        });
    }
    const optExportMd = document.getElementById('opt-export-md');
    if (optExportMd) {
        optExportMd.addEventListener('click', () => {
            closeAllPopovers();
            exportSessionMarkdown();
        });
    }
    document.getElementById('opt-clear-chat').addEventListener('click', () => {
        startNewConversation();
        closeAllPopovers();
    });
    document.getElementById('opt-export-json').addEventListener('click', () => {
        closeAllPopovers();
        exportSessionJson();
    });
    const optToggleMock = document.getElementById('opt-toggle-mock');
    const optMockStatus = document.getElementById('opt-mock-status');
    if (optToggleMock) {
        optToggleMock.addEventListener('click', () => {
            mockModeEnabled = !mockModeEnabled;
            // Persist it too, so the switch and the server agree after a reload.
            if (currentConfig) {
                currentConfig.mock_mode = mockModeEnabled;
                persistDisplayVerbosityConfig();
            }
            if (optMockStatus) optMockStatus.textContent = mockModeEnabled ? 'On' : 'Off';
            closeAllPopovers();
            showToast(mockModeEnabled ? 'Mock model on - no API calls.' : 'Mock model off - real provider.', 'info');
        });
    }

    // Projects Tree & Inline Project Creator Listeners
    if (btnNewProjectFolder) {
        btnNewProjectFolder.addEventListener('click', (e) => {
            e.stopPropagation();
            toggleInlineProjectForm();
        });
    }

    // Inline Sidebar Project Creator & Directory Browser Listeners
    if (btnCloseInlineProject) btnCloseInlineProject.addEventListener('click', () => toggleInlineProjectForm(false));
    if (btnCancelInlineProject) btnCancelInlineProject.addEventListener('click', () => toggleInlineProjectForm(false));
    if (btnSubmitInlineProject) btnSubmitInlineProject.addEventListener('click', submitInlineProject);
    if (btnBrowseInlineProject) {
        btnBrowseInlineProject.addEventListener('click', (e) => {
            e.stopPropagation();
            openFileBrowser(inlineProjectPath ? inlineProjectPath.value : '', {
                title: 'Select Project Workspace Directory',
                onSelect: (chosen) => {
                    if (inlineProjectPath) inlineProjectPath.value = chosen;
                    autoFillProjectNameFromPath(inlineProjectName, chosen);
                    if (inlineBrowserDrawer && !inlineBrowserDrawer.classList.contains('hidden')) {
                        fetchInlineDirectory(chosen);
                    }
                }
            });
        });
    }

    if (btnToggleInlineTree) {
        btnToggleInlineTree.addEventListener('click', (e) => {
            e.stopPropagation();
            if (!inlineBrowserDrawer) return;
            const isHidden = inlineBrowserDrawer.classList.contains('hidden');
            inlineBrowserDrawer.classList.toggle('hidden', !isHidden);
            btnToggleInlineTree.textContent = isHidden ? '▴' : '▾';
            if (isHidden) {
                const startPath = (inlineProjectPath && inlineProjectPath.value.trim()) || '';
                fetchInlineDirectory(startPath);
            }
        });
    }

    if (btnInlineBrowserUp) {
        btnInlineBrowserUp.addEventListener('click', (e) => {
            e.stopPropagation();
            if (parentInlineBrowsePath) fetchInlineDirectory(parentInlineBrowsePath);
        });
    }
    if (btnInlineBrowserMkdir) {
        btnInlineBrowserMkdir.addEventListener('click', (e) => {
            e.stopPropagation();
            createFolderInInlineBrowser();
        });
    }
    if (btnInlineBrowserUse) {
        btnInlineBrowserUse.addEventListener('click', (e) => {
            e.stopPropagation();
            if (currentInlineBrowsePath && inlineProjectPath) {
                inlineProjectPath.value = currentInlineBrowsePath;
                autoFillProjectNameFromPath(inlineProjectName, currentInlineBrowsePath);
                showToast(`Selected '${currentInlineBrowsePath}'.`, 'info');
            }
        });
    }

    document.querySelectorAll('#inline-icon-picker .inline-icon-choice').forEach(btn => {
        btn.addEventListener('click', () => {
            document.querySelectorAll('#inline-icon-picker .inline-icon-choice').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            selectedInlineIcon = btn.getAttribute('data-icon') || '📁';
        });
    });

    // Token Analytics & Compaction Popover Listeners
    if (btnTokenAnalytics) {
        btnTokenAnalytics.addEventListener('click', (e) => {
            e.stopPropagation();
            if (!tokenAnalyticsPopover) return;
            const isHidden = tokenAnalyticsPopover.classList.contains('hidden');
            closeAllPopovers();
            if (isHidden) {
                const rect = btnTokenAnalytics.getBoundingClientRect();
                tokenAnalyticsPopover.style.left = `${Math.max(10, rect.right - 380)}px`;
                tokenAnalyticsPopover.style.top = `${rect.bottom + 8}px`;
                tokenAnalyticsPopover.classList.remove('hidden');
            }
        });
    }

    if (btnPopoverCompactCtx) {
        btnPopoverCompactCtx.addEventListener('click', async (e) => {
            e.stopPropagation();
            await compactCurrentSession();
        });
    }

    if (btnToggleProjectsSort) {
        btnToggleProjectsSort.addEventListener('click', () => {
            allProjects.reverse();
            renderProjectsTree();
        });
    }

    if (btnCloseProject) btnCloseProject.addEventListener('click', closeNewProjectModal);
    if (btnCancelProject) btnCancelProject.addEventListener('click', closeNewProjectModal);
    if (btnSubmitProject) btnSubmitProject.addEventListener('click', submitNewProject);

    document.querySelectorAll('#project-icon-picker .icon-choice').forEach(btn => {
        btn.addEventListener('click', () => {
            document.querySelectorAll('#project-icon-picker .icon-choice').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            selectedProjectIcon = btn.getAttribute('data-icon') || '📁';
        });
    });

    // Command Palette (Ctrl+K)
    if (btnSearchSessions) {
        btnSearchSessions.addEventListener('click', (e) => {
            e.stopPropagation();
            openCommandPalette();
        });
    }
    if (paletteSearchInput) {
        paletteSearchInput.addEventListener('input', () => buildAndRenderPaletteResults(paletteSearchInput.value));
        paletteSearchInput.addEventListener('keydown', handlePaletteKeydown);
    }
    if (commandPaletteModal) {
        commandPaletteModal.addEventListener('click', (e) => {
            if (e.target === commandPaletteModal) closeCommandPalette();
        });
    }
    if (projectModal) {
        projectModal.addEventListener('click', (e) => {
            if (e.target === projectModal) closeNewProjectModal();
        });
    }

    // File Browser Listeners
    if (btnBrowseProjectPath) {
        btnBrowseProjectPath.addEventListener('click', (e) => {
            e.stopPropagation();
            openFileBrowser(projectPathInput ? projectPathInput.value : '', {
                title: 'Select Project Directory',
                onSelect: (chosen) => {
                    if (projectPathInput) projectPathInput.value = chosen;
                    autoFillProjectNameFromPath(projectNameInput, chosen);
                }
            });
        });
    }
    if (btnCloseFileBrowser) btnCloseFileBrowser.addEventListener('click', closeFileBrowser);
    if (btnCancelBrowser) btnCancelBrowser.addEventListener('click', closeFileBrowser);
    if (btnConfirmBrowserSelect) btnConfirmBrowserSelect.addEventListener('click', confirmBrowserSelection);
    if (btnBrowserBack) btnBrowserBack.addEventListener('click', goBrowserBack);
    if (btnBrowserUp) btnBrowserUp.addEventListener('click', goBrowserUp);
    if (btnBrowserRefresh) btnBrowserRefresh.addEventListener('click', refreshBrowser);
    if (btnBrowserMkdir) btnBrowserMkdir.addEventListener('click', createDirectoryInBrowser);
    if (browserDirsOnlyToggle) {
        browserDirsOnlyToggle.addEventListener('change', () => {
            renderBrowserEntries(lastBrowseData);
        });
    }
    if (browserPathInput) {
        browserPathInput.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') {
                e.preventDefault();
                fetchDirectory(browserPathInput.value.trim(), { strict: true, pushHistory: true });
            }
        });
    }
    if (fileBrowserModal) {
        fileBrowserModal.addEventListener('click', (e) => {
            if (e.target === fileBrowserModal) closeFileBrowser();
        });
    }

    // Stop Task Listeners
    if (btnTopStopSession) btnTopStopSession.addEventListener('click', stopActiveSession);
    if (btnStopPrompt) btnStopPrompt.addEventListener('click', stopActiveSession);

    // Global keyboard shortcuts
    window.addEventListener('keydown', handleGlobalKeydown);
});

function closeAllPopovers() {
    [
        attachMenuPopover,
        modelPopover,
        thinkingEffortSubmenu,
        permModePopover,
        morePopover,
        serverPopover,
        moveConvPopover,
        promptAutocompleteMenu,
        tokenAnalyticsPopover
    ].forEach(el => {
        if (el) el.classList.add('hidden');
    });
    activeModelPickerContext = null;
    hideThinkingEffortSubmenu();
    closeAllCustomDropdowns();
}

function autoResizeTextarea() {
    taskInput.style.height = 'auto';
    taskInput.style.height = `${Math.min(160, taskInput.scrollHeight)}px`;
}

// ==========================================================================
// Telemetry & Active Task / Swarm Pill
// ==========================================================================
async function fetchMetrics() {
    try {
        const res = await fetch('/api/metrics');
        if (!res.ok) return;
        const d = await res.json();
        const cpuStr = typeof d.process_cpu_pct === 'number' ? `${d.process_cpu_pct.toFixed(1)}% CPU` : '--';
        const rssStr = typeof d.process_rss_mb === 'number' ? `${d.process_rss_mb.toFixed(1)} MB` : '--';
        const ramStr = (typeof d.host_used_ram_mb === 'number' && typeof d.host_total_ram_mb === 'number')
            ? `${(d.host_used_ram_mb / 1024).toFixed(1)} / ${(d.host_total_ram_mb / 1024).toFixed(1)} GB`
            : '--';

        if (statCpu && cpuStr !== '--') statCpu.textContent = cpuStr;
        if (statRss && rssStr !== '--') statRss.textContent = rssStr;
        if (settingsStatRss && rssStr !== '--') settingsStatRss.textContent = rssStr;
        if (statHostRam && ramStr !== '--') statHostRam.textContent = ramStr;
        if (telemetryPill) {
            telemetryPill.title = `CPU: ${cpuStr} · Process RSS: ${rssStr} · Host RAM: ${ramStr}`;
        }
        if (typeof d.active_subagents === 'number') {
            activeSubagentsCount = d.active_subagents;
            updateTaskRunningIndicator();
        }
    } catch (_) {}
}

function setSendMode(mode, persist = false) {
    activeSendMode = String(mode || '').trim().toLowerCase() === 'steer' ? 'steer' : 'queue';
    if (settingsSendMode) {
        settingsSendMode.querySelectorAll('[data-send-mode]').forEach(btn => {
            btn.classList.toggle('active', btn.getAttribute('data-send-mode') === activeSendMode);
        });
    }
    updateSubmitButtonState();
    if (persist && currentConfig) {
        currentConfig.send_mode = activeSendMode;
        persistDisplayVerbosityConfig();
    }
}

function updateSubmitButtonState() {
    if (!btnSubmit) return;
    btnSubmit.classList.remove('hidden');
    btnSubmit.disabled = false;
    if (isMainSessionRunning) {
        btnSubmit.title = activeSendMode === 'steer'
            ? 'Steer running agent (Enter)'
            : 'Queue prompt for next turn (Enter)';
    } else {
        btnSubmit.title = 'Send (Enter)';
    }
}

function updateBreadcrumbSessionUI(title) {
    if (currentTaskBreadcrumb) {
        currentTaskBreadcrumb.textContent = title || 'New Conversation';
        currentTaskBreadcrumb.classList.toggle('renamable', !!currentSessionId);
    }
    if (btnRenameSession) {
        btnRenameSession.classList.toggle('hidden', !currentSessionId);
    }
}

function updateTaskRunningIndicator() {
    let localRunningAgents = 0;
    sessionSubagents.forEach(sub => {
        if (sub.status === 'running' || sub.status === 'waiting') localRunningAgents++;
    });
    const agentsRunning = Math.max(localRunningAgents, activeSubagentsCount);
    const active = isMainSessionRunning || agentsRunning > 0;

    if (taskRunningStrip) taskRunningStrip.classList.toggle('hidden', !active);
    updateSubmitButtonState();
    if (btnStopPrompt) btnStopPrompt.classList.toggle('hidden', !isMainSessionRunning);

    if (taskRunningLabel) {
        if (isMainSessionRunning && agentsRunning > 0) {
            taskRunningLabel.textContent = `1 task · ${agentsRunning} swarm running`;
        } else if (agentsRunning > 0) {
            taskRunningLabel.textContent = `${agentsRunning} swarm running`;
        } else {
            taskRunningLabel.textContent = '1 task running';
        }
    }
    renderSwarmHeader();
}

function compactPreviewText(text, maxLen = 140) {
    if (!text) return '';
    const flat = String(text).replace(/\s+/g, ' ').trim();
    if (flat.length <= maxLen) return flat;
    return `${flat.slice(0, Math.max(1, maxLen - 1))}…`;
}

function scheduleSwarmStateSync() {
    if (!currentSessionId) return;
    clearTimeout(swarmStateSyncTimer);
    swarmStateSyncTimer = setTimeout(() => {
        if (currentSessionId) fetchSessionSwarmState(currentSessionId);
    }, 180);
}

// ==========================================================================
// Right Inspector Pane
// ==========================================================================
function openInspector(tabName, autoTriggered = false) {
    if (!inspectorPane) return;
    if (autoTriggered && userClosedInspector) {
        if (inspectorActivityDot) inspectorActivityDot.classList.remove('hidden');
        return;
    }
    inspectorPane.classList.remove('collapsed');
    if (btnToggleDrawer) btnToggleDrawer.classList.add('active');
    if (inspectorActivityDot) inspectorActivityDot.classList.add('hidden');
    if (tabName) switchInspectorTab(tabName);
}

function closeInspector() {
    if (!inspectorPane) return;
    inspectorPane.classList.add('collapsed');
    if (btnToggleDrawer) btnToggleDrawer.classList.remove('active');
}

function switchInspectorTab(tabName) {
    document.querySelectorAll('.inspector-tab').forEach(t => {
        t.classList.toggle('active', t.getAttribute('data-inspector-tab') === tabName);
    });
    document.querySelectorAll('.inspector-tab-panel').forEach(p => {
        p.classList.toggle('hidden', p.id !== `inspector-tab-${tabName}`);
        p.classList.toggle('active', p.id === `inspector-tab-${tabName}`);
    });
    if (tabName === 'workspace') {
        loadWorkspaceTree();
    }
}

function switchSwarmSubtab(subtab) {
    activeSwarmSubtab = subtab || 'board';
    document.querySelectorAll('.swarm-subtab').forEach(btn => {
        btn.classList.toggle('active', btn.getAttribute('data-swarm-subtab') === activeSwarmSubtab);
    });
    document.querySelectorAll('.swarm-subview').forEach(view => {
        const match = view.id === `swarm-subview-${activeSwarmSubtab}`;
        view.classList.toggle('hidden', !match);
        view.classList.toggle('active', match);
    });
    if (activeSwarmSubtab === 'graph') renderSwarmGraph();
}

function getSwarmAvatarColors(name) {
    const key = String(name || 'agent').toLowerCase();
    if (key === 'coordinator' || key === 'root') {
        return {
            primary: '#f59e0b',
            highlight: '#fde68a',
            eye: '#451a03',
            bg: '#1b1710',
            border: 'rgba(245, 158, 11, 0.36)',
            isCoord: true,
            variant: 0
        };
    }
    let hash = 0;
    for (let i = 0; i < key.length; i++) hash = ((hash << 5) - hash + key.charCodeAt(i)) | 0;
    const palettes = [
        { primary: '#a855f7', highlight: '#e9d5ff', eye: '#1e1b4b' },
        { primary: '#38bdf8', highlight: '#bae6fd', eye: '#082f49' },
        { primary: '#34d399', highlight: '#a7f3d0', eye: '#022c22' },
        { primary: '#ec4899', highlight: '#fbcfe8', eye: '#500724' },
        { primary: '#6366f1', highlight: '#c7d2fe', eye: '#1e1b4b' },
        { primary: '#2dd4bf', highlight: '#99f6e4', eye: '#042f2e' },
        { primary: '#60a5fa', highlight: '#dbeafe', eye: '#172554' },
        { primary: '#fb923c', highlight: '#fed7aa', eye: '#431407' }
    ];
    const p = palettes[Math.abs(hash) % palettes.length];
    return {
        primary: p.primary,
        highlight: p.highlight,
        eye: p.eye,
        bg: '#151722',
        border: 'rgba(255, 255, 255, 0.1)',
        isCoord: false,
        variant: Math.abs(hash >> 3) % 4
    };
}

function buildSwarmAvatarHtml(name) {
    const clean = String(name || 'agent').replace(/^@/, '');
    const c = getSwarmAvatarColors(clean);
    const masks = [
        // 0: Crowned / antennae bot (coordinator or variant 0)
        c.isCoord
            ? [
                '..H..H..',
                '.HHHHHH.',
                '.PPPPPP.',
                'PPEPPPEP',
                'PPPPPPPP',
                '.PPPPPP.',
                '.P.PP.P.',
                '.H....H.'
            ]
            : [
                '..H..H..',
                '...PP...',
                '.PPPPPP.',
                'PPEPPPEP',
                'PPHHHHHP',
                '.PPPPPP.',
                '..P..P..',
                '.HH..HH.'
            ],
        // 1: Visor bot
        [
            '...HH...',
            '.PPPPPP.',
            'PPPPPPPP',
            'PEEEEEEP',
            'PPPPPPPP',
            '.PPHHPP.',
            '.P....P.',
            '.H....H.'
        ],
        // 2: Horned sprite bot
        [
            '.H....H.',
            '.P....P.',
            '.PPPPPP.',
            'PPEPPPEP',
            'PPPPPPPP',
            '.P.PP.P.',
            '.PP..PP.',
            '........'
        ],
        // 3: Compact blocky droid
        [
            '..HHHH..',
            '.PPPPPP.',
            'PPEPPPEP',
            'PPPPPPPP',
            'P.HHHH.P',
            '.PPPPPP.',
            '..P..P..',
            '.PP..PP.'
        ]
    ];

    const grid = masks[c.isCoord ? 0 : c.variant] || masks[0];
    let rects = '';
    for (let y = 0; y < grid.length; y++) {
        const row = grid[y];
        for (let x = 0; x < row.length; x++) {
            const ch = row[x];
            if (ch === '.') continue;
            const fill = ch === 'H' ? c.highlight : (ch === 'E' ? c.eye : c.primary);
            rects += `<rect x="${x}" y="${y}" width="1" height="1" fill="${fill}"/>`;
        }
    }

    return `<div class="swarm-avatar${c.isCoord ? ' coordinator' : ''}" style="background:${c.bg};border-color:${c.border};" title="@${escapeHtml(clean)}"><svg class="swarm-pixel-sprite" width="18" height="18" viewBox="0 0 8 8" shape-rendering="crispEdges" aria-hidden="true">${rects}</svg></div>`;
}

function formatSwarmBoardHtml(rawText) {
    const escaped = escapeHtml(rawText || '');
    return escaped
        .replace(/`([^`\n]+)`/g, '<code class="swarm-inline-code">$1</code>')
        .replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>')
        .replace(/(^|\s)(@[a-zA-Z0-9_-]+)/g, '$1<span class="swarm-inline-mention">$2</span>');
}

function formatBoardTimestamp(ts) {
    if (!ts) return '';
    const d = new Date(ts);
    if (isNaN(d.getTime())) return String(ts).slice(-8);
    return d.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

function renderSwarmHeader() {
    const list = Array.from(sessionSubagents.values());
    const runningWorkers = list.filter(s => s.status === 'running' || s.status === 'waiting').length;
    const isLive = isMainSessionRunning || runningWorkers > 0;

    if (swarmTitleLabel) {
        const title = swarmTitle || (currentTaskBreadcrumb ? currentTaskBreadcrumb.textContent : '') || 'Background Swarm';
        swarmTitleLabel.textContent = title;
        swarmTitleLabel.title = title;
    }

    if (swarmLiveIndicator) {
        swarmLiveIndicator.className = `swarm-live-pill ${isLive ? 'live' : 'idle'}`;
        const txt = swarmLiveIndicator.querySelector('.swarm-live-text');
        if (txt) txt.textContent = isLive ? 'Live' : 'Idle';
    }

    if (swarmBoardCount) {
        swarmBoardCount.textContent = String(swarmBoardMessages.length);
    }

    if (swarmMembersActiveCount) {
        swarmMembersActiveCount.textContent = String(1 + runningWorkers);
    }

    if (inspectorSwarmBadge) {
        const badgeCount = runningWorkers > 0 ? runningWorkers : list.length;
        inspectorSwarmBadge.textContent = String(badgeCount);
        inspectorSwarmBadge.classList.toggle('hidden', badgeCount === 0);
    }
}

function resetInspectorState() {
    toolCallsMap.clear();
    sessionSubagents.clear();
    swarmBoardMessages = [];
    swarmBoardSeqSet = new Set();
    sessionSwarmJobs = [];
    sessionSwarmSpecialists = [];
    swarmTitle = '';
    swarmCoordinator = null;
    clearTimeout(swarmStateSyncTimer);
    sessionArtifacts = [];
    browserSnapshots = [];
    selectedSnapshotIndex = -1;
    userClosedInspector = false;

    renderSwarmHeader();
    renderInspectorSwarmTags();
    renderInspectorSwarm();
    renderInspectorBoard();
    renderSwarmGraph();
    renderInspectorArtifacts();
    renderBrowserSnapshotViewer();
    if (workspaceFileViewer) workspaceFileViewer.classList.add('hidden');
    if (inspectorActivityDot) inspectorActivityDot.classList.add('hidden');
    updateTaskRunningIndicator();
}

async function fetchSessionSwarmState(sessionId) {
    if (!sessionId) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}/swarm`);
        if (!res.ok) return;
        const data = await res.json();

        if (typeof data.swarm_title === 'string' && data.swarm_title.trim()) {
            swarmTitle = data.swarm_title.trim();
        }
        if (data.coordinator && typeof data.coordinator === 'object') {
            swarmCoordinator = data.coordinator;
        }

        if (Array.isArray(data.jobs)) {
            sessionSwarmJobs = data.jobs;
        }
        if (Array.isArray(data.specialists)) {
            sessionSwarmSpecialists = data.specialists;
        }
        renderInspectorSwarmTags();

        if (Array.isArray(data.subagents)) {
            data.subagents.forEach(info => {
                const existing = sessionSubagents.get(info.id) || { traces: [] };
                sessionSubagents.set(info.id, {
                    ...existing,
                    id: info.id,
                    role: info.role,
                    task: compactPreviewText(info.task || existing.task || '', 120),
                    parent_id: info.parent_id,
                    status: info.status,
                    job_tag: info.job_tag || existing.job_tag || null,
                    specialist: info.specialist || existing.specialist || null,
                    model: info.model || existing.model || null,
                    last_seq: info.last_seq != null ? info.last_seq : (existing.last_seq ?? null),
                    summary: compactPreviewText(info.summary || existing.summary || '', 140),
                    created_at: info.created_at
                });
            });
            renderInspectorSwarm();
            updateTaskRunningIndicator();
        }

        if (Array.isArray(data.board_messages)) {
            data.board_messages.forEach(m => addSwarmBoardMessage(m, false));
            renderInspectorBoard();
        }

        renderSwarmHeader();
        if (activeSwarmSubtab === 'graph') renderSwarmGraph();

        if (Array.isArray(data.artifacts)) {
            data.artifacts.forEach(a => addArtifactRecord(a, false));
            renderInspectorArtifacts();
        }
    } catch (_) {}
}

function renderInspectorSwarmTags() {
    const hasJobs = Array.isArray(sessionSwarmJobs) && sessionSwarmJobs.length > 0;
    const hasSpecs = Array.isArray(sessionSwarmSpecialists) && sessionSwarmSpecialists.length > 0;
    if (inspectorSwarmTagsSection) {
        inspectorSwarmTagsSection.classList.toggle('hidden', !hasJobs && !hasSpecs);
    }

    if (inspectorSwarmJobs) {
        if (!hasJobs) {
            inspectorSwarmJobs.innerHTML = '<span class="inspector-tags-empty">No jobs tagged</span>';
        } else {
            inspectorSwarmJobs.innerHTML = sessionSwarmJobs.map(job => {
                const st = (job.status || 'open').toLowerCase();
                const stClass = st === 'done' ? 'done' : (st === 'in_progress' ? 'in_progress' : 'open');
                const label = compactPreviewText(job.title || job.id || 'job', 36);
                const tip = job.description ? `${job.id}: ${job.description}` : (job.title || job.id || '');
                return `<span class="swarm-tag-pill job" title="${escapeHtml(tip)}"><span class="swarm-tag-kind">(job)</span><span>${escapeHtml(label)}</span><span class="swarm-tag-status ${stClass}">${escapeHtml(st)}</span></span>`;
            }).join('');
        }
    }

    if (inspectorSwarmSpecialists) {
        if (!hasSpecs) {
            inspectorSwarmSpecialists.innerHTML = '<span class="inspector-tags-empty">No specialists</span>';
        } else {
            inspectorSwarmSpecialists.innerHTML = sessionSwarmSpecialists.map(spec => {
                const name = compactPreviewText(spec.name || spec.id || 'specialist', 28);
                const ro = spec.default_read_only ? ' · RO' : '';
                const tip = spec.specialty || spec.name || spec.id || '';
                return `<span class="swarm-tag-pill specialist" title="${escapeHtml(tip)}"><span class="swarm-tag-kind">(member ${escapeHtml(name)})</span>${ro ? `<span>${escapeHtml(ro)}</span>` : ''}</span>`;
            }).join('');
        }
    }
}

function buildMemberRowHtml(member, isRetired) {
    const isCoord = member.id === 'coordinator';
    const isActive = !isCoord && (member.status === 'running' || member.status === 'waiting');
    const lastMsg = swarmBoardMessages.slice().reverse().find(m => m.sender === member.id);
    const lastSeq = member.last_seq != null ? member.last_seq : (lastMsg ? lastMsg.seq : null);
    const modelLabel = member.model || (currentConfig ? currentConfig.model : '') || '';
    const modelShort = modelLabel.includes('/') ? modelLabel.split('/').pop() : modelLabel;
    const metaRight = [modelShort, lastSeq != null ? `#${lastSeq}` : null].filter(Boolean).join(' · ');
    const roleOrTask = isCoord
        ? 'Coordinator'
        : compactPreviewText(member.task || member.role || '', 68);
    const digest = compactPreviewText(
        (lastMsg && lastMsg.content) || member.summary || member.task || (isCoord ? 'Orchestrating swarm' : ''),
        160
    );
    const tagTitles = [
        member.server_id ? `Server: ${member.server_id}` : null,
        member.job_tag ? `(job) ${member.job_tag}` : null,
        member.specialist ? `(member ${member.specialist})` : null
    ].filter(Boolean).join(' · ');
    const serverChipHtml = member.server_id
        ? `<span class="subagent-server-badge" title="Executed on ${escapeHtml(member.server_id)}">🖥 ${escapeHtml(member.server_id)}</span>`
        : '';
    const tagsHtml = (!isRetired && (member.server_id || member.job_tag || member.specialist))
        ? `<div class="swarm-member-tags">
            ${serverChipHtml}
            ${member.job_tag ? `<span class="swarm-inline-tag job">(job) ${escapeHtml(compactPreviewText(member.job_tag, 24))}</span>` : ''}
            ${member.specialist ? `<span class="swarm-inline-tag specialist">(member ${escapeHtml(compactPreviewText(member.specialist, 24))})</span>` : ''}
           </div>`
        : '';

    return `
        ${buildSwarmAvatarHtml(member.id)}
        <div class="swarm-member-main"${tagTitles ? ` title="${escapeHtml(tagTitles)}"` : ''}>
            <div class="swarm-member-top">
                <div class="swarm-member-ident">
                    <span class="swarm-member-handle">@${escapeHtml(member.id)}</span>
                    <span class="swarm-member-role" title="${escapeHtml(member.task || member.role || '')}">${escapeHtml(roleOrTask)}</span>
                </div>
                <div class="swarm-member-meta">
                    ${metaRight ? `<span title="${escapeHtml(modelLabel)}">${escapeHtml(metaRight)}</span>` : ''}
                    ${isActive ? `<button type="button" class="subagent-stop-btn" data-stop-sub="${escapeHtml(member.id)}">Stop</button>` : ''}
                </div>
            </div>
            ${digest ? `<div class="swarm-member-digest" title="${escapeHtml(digest)}">${escapeHtml(digest)}</div>` : ''}
            ${tagsHtml}
        </div>
    `;
}

function renderInspectorSwarm() {
    renderSwarmHeader();
    if (!swarmMembersActive || !swarmMembersRetired) {
        if (inspectorSubagentsList) inspectorSubagentsList.innerHTML = '';
        return;
    }

    const q = (swarmMembersSearch ? swarmMembersSearch.value : '').trim().toLowerCase();
    const list = Array.from(sessionSubagents.values());
    const coord = swarmCoordinator || {
        id: 'coordinator',
        role: 'Coordinator',
        model: (currentConfig && currentConfig.model) || '',
        status: isMainSessionRunning ? 'running' : 'completed',
        summary: swarmTitle || 'Swarm coordinator',
        last_seq: null
    };

    const matchesFilter = (m) => {
        if (!q) return true;
        return [m.id, m.role, m.task, m.summary, m.job_tag, m.specialist, m.model, m.server_id]
            .filter(Boolean)
            .some(v => String(v).toLowerCase().includes(q));
    };

    const activeWorkers = list.filter(s => s.status === 'running' || s.status === 'waiting');
    const retiredWorkers = list.filter(s => s.status !== 'running' && s.status !== 'waiting');

    const activeItems = [];
    if (matchesFilter(coord)) activeItems.push(coord);
    activeWorkers.filter(matchesFilter).forEach(w => activeItems.push(w));

    swarmMembersActive.innerHTML = '';
    activeItems.forEach(member => {
        const row = document.createElement('div');
        row.className = 'swarm-member-row';
        row.innerHTML = buildMemberRowHtml(member, false);
        const stopBtn = row.querySelector('[data-stop-sub]');
        if (stopBtn) {
            stopBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                stopSubagent(member.id);
            });
        }
        row.addEventListener('click', () => {
            if (swarmBoardSearch) swarmBoardSearch.value = `@${member.id}`;
            switchSwarmSubtab('board');
            renderInspectorBoard();
        });
        swarmMembersActive.appendChild(row);
    });

    const filteredRetired = retiredWorkers.filter(matchesFilter);
    if (swarmMembersRetiredSection) {
        swarmMembersRetiredSection.classList.toggle('hidden', retiredWorkers.length === 0 || (q && filteredRetired.length === 0));
    }
    if (swarmRetiredCount) {
        swarmRetiredCount.textContent = String(retiredWorkers.length);
    }

    swarmMembersRetired.innerHTML = '';
    filteredRetired.forEach(member => {
        const row = document.createElement('div');
        row.className = 'swarm-member-row retired';
        row.innerHTML = buildMemberRowHtml(member, true);
        row.addEventListener('click', () => {
            if (swarmBoardSearch) swarmBoardSearch.value = `@${member.id}`;
            switchSwarmSubtab('board');
            renderInspectorBoard();
        });
        swarmMembersRetired.appendChild(row);
    });

    if (activeSwarmSubtab === 'graph') renderSwarmGraph();
}

async function stopSubagent(subId) {
    if (!currentSessionId || !subId) return;
    let ok = false;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/subagents/${encodeURIComponent(subId)}/stop`, {
            method: 'POST'
        });
        ok = res.ok;
    } catch (e) {
        console.error('Failed to stop subagent', e);
    }
    if (ok) {
        const sub = sessionSubagents.get(subId);
        if (sub) {
            sub.status = 'stopped';
            renderInspectorSwarm();
            updateTaskRunningIndicator();
        }
        return;
    }
    // A failed stop used to vanish into the console while the swarm pill stayed
    // green and the member kept counting as active. Surface it and mark the
    // member as interrupted so the count cannot lie.
    const sub = sessionSubagents.get(subId);
    if (sub) {
        sub.status = 'stopped';
        renderInspectorSwarm();
        updateTaskRunningIndicator();
    }
    showToast(
        'Could not stop this subagent (the server did not confirm it). ' +
        'It may still be running - try Stop again.',
        'error', 9000
    );
}

function addSwarmBoardMessage(msg, shouldRender = true) {
    if (!msg || swarmBoardSeqSet.has(msg.seq)) return;
    swarmBoardSeqSet.add(msg.seq);
    swarmBoardMessages.push(msg);
    if (swarmBoardMessages.length > 200) {
        const dropped = swarmBoardMessages.shift();
        if (dropped) swarmBoardSeqSet.delete(dropped.seq);
    }
    if (shouldRender) {
        renderInspectorBoard();
        renderSwarmHeader();
    }
}

function renderInspectorBoard() {
    if (!inspectorBoardMessages) return;
    renderSwarmHeader();

    const q = (swarmBoardSearch ? swarmBoardSearch.value : '').trim().toLowerCase();
    let msgs = swarmBoardMessages;

    if (swarmBoardFilterMode === 'mentions') {
        msgs = msgs.filter(m => m.target || (m.content && m.content.includes('@')));
    } else if (swarmBoardFilterMode === 'tagged') {
        msgs = msgs.filter(m => m.job_tag || m.specialist);
    }

    if (q) {
        msgs = msgs.filter(m =>
            String(m.sender || '').toLowerCase().includes(q) ||
            `@` + String(m.sender || '').toLowerCase() === q ||
            String(m.target || '').toLowerCase().includes(q) ||
            `@` + String(m.target || '').toLowerCase() === q ||
            String(m.content || '').toLowerCase().includes(q) ||
            String(m.job_tag || '').toLowerCase().includes(q) ||
            String(m.specialist || '').toLowerCase().includes(q)
        );
    }

    if (msgs.length === 0) {
        const emptyText = swarmBoardMessages.length === 0
            ? 'Shared board messages will stream here in real time.'
            : 'No board messages match the current search or filter.';
        inspectorBoardMessages.innerHTML = `<div class="inspector-empty">${escapeHtml(emptyText)}</div>`;
        return;
    }

    const wasNearBottom = inspectorBoardMessages.scrollHeight - inspectorBoardMessages.scrollTop - inspectorBoardMessages.clientHeight < 48;
    inspectorBoardMessages.innerHTML = '';
    msgs.forEach(m => {
        const item = document.createElement('div');
        item.className = 'board-msg-item';
        const rawTarget = String(m.target || '').trim();
        const targetsList = rawTarget
            ? rawTarget.split(/[\s,]+/).map(t => t.replace(/^@/, '')).filter(Boolean)
            : [];
        const targetHtml = targetsList.length > 0
            ? `<span class="board-msg-arrow">→</span><span class="board-msg-targets">${targetsList.map(t => `<span class="board-msg-target">@${escapeHtml(t)}</span>`).join(' ')}</span>`
            : '';
        const jobHtml = m.job_tag ? `<span class="swarm-inline-tag job">(job) ${escapeHtml(compactPreviewText(m.job_tag, 20))}</span>` : '';
        const specHtml = m.specialist ? `<span class="swarm-inline-tag specialist">(member ${escapeHtml(compactPreviewText(m.specialist, 20))})</span>` : '';
        const timeStr = formatBoardTimestamp(m.timestamp);
        const bodyHtml = formatSwarmBoardHtml(m.content || '');
        const isLongBody = (m.content || '').split('\n').length > 4 || (m.content || '').length > 260;
        item.innerHTML = `
            ${buildSwarmAvatarHtml(m.sender)}
            <div class="board-msg-content-col">
                <div class="board-msg-meta">
                    <span class="board-msg-sender" data-filter-sender="@${escapeHtml(m.sender)}" title="Filter by @${escapeHtml(m.sender)}">@${escapeHtml(m.sender)}</span>
                    ${targetHtml}
                    ${jobHtml}
                    ${specHtml}
                    <span class="board-msg-time">#${m.seq}${timeStr ? ` · ${escapeHtml(timeStr)}` : ''}</span>
                </div>
                <div class="board-msg-body${isLongBody ? ' clampable' : ''}" title="${isLongBody ? 'Click to expand / collapse' : ''}">${bodyHtml}</div>
            </div>
        `;
        const senderEl = item.querySelector('[data-filter-sender]');
        if (senderEl && swarmBoardSearch) {
            senderEl.addEventListener('click', (e) => {
                e.stopPropagation();
                const handle = senderEl.getAttribute('data-filter-sender') || '';
                swarmBoardSearch.value = swarmBoardSearch.value.trim() === handle ? '' : handle;
                renderInspectorBoard();
            });
        }
        const bodyEl = item.querySelector('.board-msg-body');
        if (bodyEl) {
            bodyEl.addEventListener('click', () => bodyEl.classList.toggle('expanded'));
        }
        inspectorBoardMessages.appendChild(item);
    });
    if (wasNearBottom || !q) {
        inspectorBoardMessages.scrollTop = inspectorBoardMessages.scrollHeight;
    }
}

function renderSwarmGraph() {
    if (!swarmGraphCanvas) return;
    const workers = Array.from(sessionSubagents.values());
    if (workers.length === 0) {
        swarmGraphCanvas.innerHTML = '<div class="inspector-empty">Spawn background workers to inspect the Swarm Graph.</div>';
        return;
    }

    const width = Math.max(swarmGraphCanvas.clientWidth - 20, 340);
    const coordY = 38;
    const cols = Math.max(1, Math.min(3, workers.length));
    const rows = Math.ceil(workers.length / cols);
    const rowGap = 94;
    const height = Math.max(250, coordY + 75 + rows * rowGap);
    const coordX = width / 2;

    const statusColor = (status) => {
        if (status === 'running') return '#22c55e';
        if (status === 'waiting') return '#f59e0b';
        if (status === 'failed' || status === 'stopped') return '#ef4444';
        return '#8b5cf6';
    };

    const nodePositions = new Map();
    nodePositions.set('coordinator', { x: coordX, y: coordY });
    nodePositions.set('root', { x: coordX, y: coordY });

    workers.forEach((w, idx) => {
        const r = Math.floor(idx / cols);
        const c = idx % cols;
        const rowCount = r === rows - 1 ? (workers.length - r * cols) : cols;
        const marginX = 56;
        const usableW = Math.max(120, width - marginX * 2);
        const stepX = rowCount === 1 ? 0 : usableW / (rowCount - 1);
        const x = rowCount === 1 ? width / 2 : marginX + c * stepX;
        const y = coordY + 106 + r * rowGap;
        nodePositions.set(w.id, { x, y });
    });

    let edgesSvg = '';
    workers.forEach(w => {
        const pKey = w.parent_id && nodePositions.has(w.parent_id) ? w.parent_id : 'coordinator';
        const pPos = nodePositions.get(pKey);
        const cPos = nodePositions.get(w.id);
        if (!pPos || !cPos) return;
        const midY = (pPos.y + cPos.y) / 2;
        const stroke = w.status === 'running' ? 'rgba(34,197,94,0.6)' : 'rgba(148,163,184,0.35)';
        edgesSvg += `<path d="M ${pPos.x} ${pPos.y + 18} C ${pPos.x} ${midY}, ${cPos.x} ${midY}, ${cPos.x} ${cPos.y - 22}" fill="none" stroke="${stroke}" stroke-width="1.6" stroke-dasharray="${w.status === 'running' ? '4 3' : 'none'}"/>`;
        if (w.job_tag) {
            const lx = (pPos.x + cPos.x) / 2;
            const ly = midY - 3;
            edgesSvg += `<text x="${lx}" y="${ly}" text-anchor="middle" fill="#9ca3af" font-size="9" font-family="ui-monospace, monospace">${escapeHtml(compactPreviewText(w.job_tag, 16))}</text>`;
        }
    });

    const coordLive = isMainSessionRunning ? 'running' : 'completed';
    let nodesSvg = `
        <g transform="translate(${coordX}, ${coordY})">
            <rect x="-56" y="-17" width="112" height="34" rx="9" fill="#171923" stroke="${statusColor(coordLive)}" stroke-width="1.5"/>
            <text x="0" y="-2" text-anchor="middle" fill="#f9fafb" font-size="11" font-weight="600" font-family="ui-monospace, monospace">@coordinator</text>
            <text x="0" y="10" text-anchor="middle" fill="#fbbf24" font-size="9">${isMainSessionRunning ? 'active' : 'coordinator'}</text>
        </g>
    `;

    workers.forEach(w => {
        const pos = nodePositions.get(w.id);
        if (!pos) return;
        const col = statusColor(w.status);
        const shortId = `@${compactPreviewText(w.id, 15)}`;
        const subLbl = compactPreviewText(w.role || w.status, 17);
        nodesSvg += `
            <g class="swarm-graph-node" data-graph-id="${escapeHtml(w.id)}" transform="translate(${pos.x}, ${pos.y})" style="cursor:pointer;">
                <rect x="-54" y="-20" width="108" height="40" rx="9" fill="#131520" stroke="${col}" stroke-width="1.4"/>
                <text x="0" y="-4" text-anchor="middle" fill="#f3f4f6" font-size="10.5" font-weight="600" font-family="ui-monospace, monospace">${escapeHtml(shortId)}</text>
                <text x="0" y="9" text-anchor="middle" fill="${col}" font-size="9">${escapeHtml(subLbl)}</text>
            </g>
        `;
    });

    swarmGraphCanvas.innerHTML = `<svg class="swarm-graph-svg" viewBox="0 0 ${width} ${height}">${edgesSvg}${nodesSvg}</svg>`;
    swarmGraphCanvas.querySelectorAll('[data-graph-id]').forEach(el => {
        el.addEventListener('click', () => {
            const id = el.getAttribute('data-graph-id');
            if (id && swarmBoardSearch) {
                swarmBoardSearch.value = `@${id}`;
                switchSwarmSubtab('board');
                renderInspectorBoard();
            }
        });
    });
}

async function postSwarmMessage() {
    if (!currentSessionId) {
        showToast('Start or select a session before posting to the swarm board.', 'warning');
        return;
    }
    const content = swarmPostInput ? swarmPostInput.value.trim() : '';
    const targetRaw = swarmPostTarget ? swarmPostTarget.value.trim().replace(/^@/, '') : '';
    if (!content) return;

    try {
        if (btnSwarmPostSend) btnSwarmPostSend.disabled = true;
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/swarm/post`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                sender: 'user',
                target: targetRaw || null,
                content
            })
        });
        if (res.ok) {
            const msg = await res.json();
            addSwarmBoardMessage(msg, true);
            if (swarmPostInput) swarmPostInput.value = '';
        }
    } catch (e) {
        console.error('Failed to post swarm message', e);
    } finally {
        if (btnSwarmPostSend) btnSwarmPostSend.disabled = false;
    }
}

function formatShortTime(ts) {
    if (!ts) return '';
    const d = new Date(ts);
    if (isNaN(d.getTime())) return String(ts).slice(-8);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

// ==========================================================================
// Tab 2: Browser & UI Snapshot Inspector
// ==========================================================================
function addBrowserSnapshot(snap) {
    if (!snap) return;
    browserSnapshots.push({
        url: snap.url || '',
        title: snap.title || snap.url || 'Browser Snapshot',
        screenshot_url: snap.screenshot_url || null,
        console_errors: Array.isArray(snap.console_errors) ? snap.console_errors : [],
        dom_summary: snap.dom_summary || ''
    });
    selectedSnapshotIndex = browserSnapshots.length - 1;
    renderBrowserSnapshotViewer();
}

function renderBrowserSnapshotViewer() {
    if (!browserSnapshotViewer) return;

    if (inspectorBrowserBadge) {
        inspectorBrowserBadge.textContent = String(browserSnapshots.length);
        inspectorBrowserBadge.classList.toggle('hidden', browserSnapshots.length === 0);
    }

    if (browserSnapshots.length === 0) {
        if (browserConsoleBadge) browserConsoleBadge.classList.add('hidden');
        if (browserSnapshotGallery) browserSnapshotGallery.classList.add('hidden');
        browserSnapshotViewer.innerHTML = '<div class="inspector-empty">Run <code>browser_ui</code> or click Audit to capture a live screenshot, console logs, and DOM summary.</div>';
        return;
    }

    const snap = browserSnapshots[selectedSnapshotIndex] || browserSnapshots[browserSnapshots.length - 1];
    const errCount = snap.console_errors ? snap.console_errors.length : 0;

    if (browserConsoleBadge) {
        browserConsoleBadge.classList.remove('hidden');
        browserConsoleBadge.className = `console-badge ${errCount > 0 ? 'err' : 'ok'}`;
        browserConsoleBadge.textContent = `${errCount} error${errCount === 1 ? '' : 's'}`;
    }

    if (browserTestUrl && snap.url) {
        browserTestUrl.value = snap.url;
    }

    const imgUrl = snap.screenshot_url ? appendSessionQuery(snap.screenshot_url) : null;
    let html = `
        <div class="snapshot-meta-card">
            <div class="snapshot-title">${escapeHtml(snap.title || 'Untitled Page')}</div>
            <div class="snapshot-url">${escapeHtml(snap.url)}</div>
            ${snap.dom_summary ? `<div class="browser-dom-summary">${escapeHtml(snap.dom_summary)}</div>` : ''}
        </div>
    `;

    if (imgUrl) {
        html += `<img src="${escapeHtml(imgUrl)}" alt="Browser Screenshot" class="snapshot-img-frame" id="inspector-active-shot">`;
    }

    if (errCount > 0) {
        html += `<div class="snapshot-errors-list">${snap.console_errors.map(e => `<div>• ${escapeHtml(e)}</div>`).join('')}</div>`;
    }

    browserSnapshotViewer.innerHTML = html;

    const shotEl = document.getElementById('inspector-active-shot');
    if (shotEl && imgUrl) {
        shotEl.addEventListener('click', () => openImageLightbox(imgUrl, snap.title || snap.url));
    }

    if (browserSnapshotGallery) {
        const shotsWithImg = browserSnapshots
            .map((s, idx) => ({ s, idx }))
            .filter(item => item.s.screenshot_url);
        if (shotsWithImg.length > 1) {
            browserSnapshotGallery.classList.remove('hidden');
            browserSnapshotGallery.innerHTML = '';
            shotsWithImg.forEach(({ s, idx }) => {
                const thumb = document.createElement('img');
                thumb.src = appendSessionQuery(s.screenshot_url);
                thumb.alt = s.title || s.url;
                thumb.title = s.url;
                thumb.className = `gallery-thumb ${idx === selectedSnapshotIndex ? 'active' : ''}`;
                thumb.addEventListener('click', () => {
                    selectedSnapshotIndex = idx;
                    renderBrowserSnapshotViewer();
                });
                browserSnapshotGallery.appendChild(thumb);
            });
        } else {
            browserSnapshotGallery.classList.add('hidden');
        }
    }
}

function appendSessionQuery(url) {
    if (!url) return url;
    if (!currentSessionId || url.includes('session_id=')) return url;
    return url + (url.includes('?') ? '&' : '?') + `session_id=${encodeURIComponent(currentSessionId)}`;
}

function openImageLightbox(src, title) {
    if (!imageLightboxModal || !lightboxImg) return;
    lightboxImg.src = src;
    if (lightboxTitle) lightboxTitle.textContent = title || 'Browser UI Screenshot';
    imageLightboxModal.classList.remove('hidden');
}

// ==========================================================================
// Tab 3: Workspace Tree & Artifacts Inspector
// ==========================================================================
function addArtifactRecord(artifact, shouldRender = true) {
    if (!artifact) return;
    const exists = sessionArtifacts.some(a => a.id === artifact.id || (a.path && a.path === artifact.path));
    if (!exists) {
        sessionArtifacts.push(artifact);
    }
    if (shouldRender) renderInspectorArtifacts();
}

function renderInspectorArtifacts() {
    if (!inspectorArtifactsList) return;
    if (inspectorArtifactsCount) inspectorArtifactsCount.textContent = String(sessionArtifacts.length);
    if (inspectorFilesBadge) {
        inspectorFilesBadge.textContent = String(sessionArtifacts.length);
        inspectorFilesBadge.classList.toggle('hidden', sessionArtifacts.length === 0);
    }

    if (sessionArtifacts.length === 0) {
        inspectorArtifactsList.innerHTML = '<div class="inspector-empty">Artifacts created via <code>create_artifact</code> appear here.</div>';
        return;
    }

    inspectorArtifactsList.innerHTML = '';
    sessionArtifacts.forEach(art => {
        const row = document.createElement('div');
        row.className = 'inspector-artifact-item';
        row.innerHTML = `
            <span class="inspector-artifact-title" title="${escapeHtml(art.path)}">${escapeHtml(art.title)}</span>
            <span class="inspector-artifact-kind">${escapeHtml(art.kind || 'artifact')}</span>
        `;
        row.addEventListener('click', () => inspectArtifact(art));
        inspectorArtifactsList.appendChild(row);
    });
}

async function inspectArtifact(artifact) {
    openInspector('workspace');
    if (!artifact) return;
    const rawPath = (artifact.path || '').replace(/^\.?harness_artifacts[\\/]/, '');

    if (rawPath) {
        try {
            const url = appendSessionQuery(`/api/artifacts/${rawPath.split(/[\\/]/).map(encodeURIComponent).join('/')}`);
            const res = await fetch(url);
            if (res.ok) {
                const ct = res.headers.get('content-type') || '';
                if (ct.startsWith('image/')) {
                    openImageLightbox(url, artifact.title);
                    return;
                }
                const text = await res.text();
                showFileInViewer(artifact.path, artifact.kind === 'markdown' || artifact.path.endsWith('.md') ? 'markdown' : 'text', text);
                return;
            }
        } catch (_) {}
    }

    // Fallback to preview text or workspace file endpoint
    if (artifact.path) {
        const ok = await openWorkspaceFile(artifact.path);
        if (ok) return;
    }
    showFileInViewer(artifact.title, 'markdown', artifact.preview || '');
}

async function loadWorkspaceTree() {
    if (!workspaceTreeList) return;
    workspaceTreeList.innerHTML = '<div class="inspector-empty">Loading workspace tree...</div>';
    try {
        let url = '/api/workspace/tree?depth=3';
        if (currentSessionId) url += `&session_id=${encodeURIComponent(currentSessionId)}`;
        const res = await fetch(url);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();
        if (workspaceRootLabel) {
            const parts = (data.root || '.').split(/[\\/]/).filter(Boolean);
            workspaceRootLabel.textContent = parts[parts.length - 1] || data.root || '.';
            workspaceRootLabel.title = data.root || '';
        }
        renderWorkspaceTreeNodes(data.entries || [], workspaceTreeList);
    } catch (e) {
        workspaceTreeList.innerHTML = `<div class="inspector-empty" style="color:var(--accent-red);">Failed to load tree: ${escapeHtml(e.message)}</div>`;
    }
}

function renderWorkspaceTreeNodes(entries, container) {
    container.innerHTML = '';
    if (!entries || entries.length === 0) {
        container.innerHTML = '<div class="inspector-empty">(Empty workspace)</div>';
        return;
    }
    entries.forEach(entry => container.appendChild(createWorkspaceTreeNode(entry)));
}

function createWorkspaceTreeNode(entry) {
    const node = document.createElement('div');
    node.className = 'ws-tree-node collapsed';

    const row = document.createElement('div');
    row.className = 'ws-tree-row';
    const icon = entry.is_dir ? '▸ 📁' : '📄';
    const sizeText = !entry.is_dir && typeof entry.size_bytes === 'number' ? formatBytes(entry.size_bytes) : '';
    row.innerHTML = `
        <span>${icon}</span>
        <span class="ws-tree-name" title="${escapeHtml(entry.path)}">${escapeHtml(entry.name)}</span>
        ${sizeText ? `<span class="ws-tree-size">${sizeText}</span>` : ''}
    `;

    node.appendChild(row);

    if (entry.is_dir) {
        const childrenWrap = document.createElement('div');
        childrenWrap.className = 'ws-tree-children';
        if (Array.isArray(entry.children) && entry.children.length > 0) {
            entry.children.forEach(child => childrenWrap.appendChild(createWorkspaceTreeNode(child)));
        }
        node.appendChild(childrenWrap);
        row.addEventListener('click', () => {
            node.classList.toggle('collapsed');
            const isCollapsed = node.classList.contains('collapsed');
            row.firstElementChild.textContent = isCollapsed ? '▸ 📁' : '▾ 📂';
        });
    } else {
        row.addEventListener('click', () => {
            document.querySelectorAll('.ws-tree-row.active').forEach(el => el.classList.remove('active'));
            row.classList.add('active');
            openWorkspaceFile(entry.path);
        });
    }
    return node;
}

async function openWorkspaceFile(path) {
    if (!path) return false;
    try {
        let url = `/api/workspace/file?path=${encodeURIComponent(path)}`;
        if (currentSessionId) url += `&session_id=${encodeURIComponent(currentSessionId)}`;
        const res = await fetch(url);
        if (!res.ok) return false;
        const data = await res.json();
        if (data.is_binary) {
            showFileInViewer(data.path || path, 'binary', `[Binary file: ${formatBytes(data.size_bytes || 0)}]`);
        } else {
            const content = data.truncated
                ? `${data.content}\n\n... [Truncated at 200 KB; total size ${formatBytes(data.size_bytes || 0)}]`
                : (data.content || '');
            showFileInViewer(data.path || path, data.language || 'text', content);
        }
        return true;
    } catch (_) {
        return false;
    }
}

function showFileInViewer(path, language, content) {
    if (!workspaceFileViewer) return;
    currentViewerContent = content || '';
    currentViewerIsMarkdown = language === 'markdown' || String(path).toLowerCase().endsWith('.md');
    currentViewerPreviewMd = currentViewerIsMarkdown;

    if (fileViewerPath) fileViewerPath.textContent = path;
    if (fileViewerLang) fileViewerLang.textContent = language || 'text';
    if (btnToggleMdPreview) {
        btnToggleMdPreview.classList.toggle('hidden', !currentViewerIsMarkdown);
        btnToggleMdPreview.textContent = currentViewerPreviewMd ? 'Raw' : 'Preview';
    }
    renderFileViewerContent();
    workspaceFileViewer.classList.remove('hidden');
}

function renderFileViewerContent() {
    if (!fileViewerBody) return;
    if (currentViewerIsMarkdown && currentViewerPreviewMd) {
        fileViewerBody.classList.add('markdown-view');
        fileViewerBody.innerHTML = `<div class="assistant-msg-content">${renderMarkdown(currentViewerContent)}</div>`;
    } else {
        fileViewerBody.classList.remove('markdown-view');
        fileViewerBody.textContent = currentViewerContent;
    }
    if (btnToggleMdPreview) {
        btnToggleMdPreview.textContent = currentViewerPreviewMd ? 'Raw' : 'Preview';
    }
}

// ==========================================================================
// Unified Settings Hub & Customizations (Skills / Permissions / Tools)
// ==========================================================================
let activeSettingsTab = 'memory';

function openSettingsModal(tabName = 'memory') {
    closeAllPopovers();
    if (settingsSaveFeedback) settingsSaveFeedback.textContent = '';
    if (configModal) configModal.classList.remove('hidden');
    switchSettingsTab(tabName);
}

/// Every settings page, discovered from the DOM rather than a hardcoded list.
///
/// The list used to be literals in this function, so adding a settings page
/// (Security) made the rail render a tab that silently fell back to `memory`
/// on click: the button moved, nothing else did. Deriving the set from the
/// markup keeps the two in step, and the unit test below asserts they match.
function settingsTabNames() {
    const names = [];
    document.querySelectorAll('.settings-nav-item[data-settings-tab]').forEach(btn => {
        const name = btn.getAttribute('data-settings-tab');
        if (name && !names.includes(name)) names.push(name);
    });
    if (!names.includes('memory')) names.unshift('memory');
    return names;
}

function switchSettingsTab(tabName = 'memory') {
    const validTabs = settingsTabNames();
    const tab = validTabs.includes(tabName) ? tabName : 'memory';
    activeSettingsTab = tab;

    document.querySelectorAll('.settings-nav-item[data-settings-tab]').forEach(btn => {
        btn.classList.toggle('active', btn.getAttribute('data-settings-tab') === tab);
    });
    validTabs.forEach(t => {
        const panel = document.getElementById(`settings-page-${t}`);
        if (panel) panel.classList.toggle('hidden', t !== tab);
    });

    if (btnSaveConfig) btnSaveConfig.classList.toggle('hidden', tab !== 'memory');
    if (btnSavePermissions) btnSavePermissions.classList.toggle('hidden', tab !== 'tools');

    if (tab === 'memory') {
        fetchMetrics();
        if (settingsCompactStatus) {
            settingsCompactStatus.textContent = currentSessionId
                ? `Active session: ${currentSessionId.slice(0, 8)}…`
                : 'Send or open a session to compact context.';
        }
    } else if (tab === 'providers') {
        renderConfiguredProviders();
    } else if (tab === 'orchestrator') {
        syncOrchestratorControls();
    } else if (tab === 'servers') {
        renderConfiguredServers();
        probeAllServers();
    } else if (tab === 'mcp') {
        loadMcpServers();
    } else if (tab === 'search') {
        syncSearchModalUI();
    } else if (tab === 'tools') {
        syncPermissionControls();
        if (settingsToolsCountBadge) {
            if (Array.isArray(cachedToolsList) && cachedToolsList.length > 0) {
                settingsToolsCountBadge.textContent = `${cachedToolsList.length} tools`;
            } else {
                loadTools();
            }
        }
    } else if (tab === 'skills') {
        fetchSkills();
    } else if (tab === 'security') {
        syncPrivilegedShellUi(currentConfig && currentConfig.allow_privileged_shell === true);
    }
}

function openCustomizationsModal(defaultTab = 'skills') {
    openSettingsModal(defaultTab === 'permissions' ? 'tools' : 'skills');
}

function switchCustomizationsTab(tab) {
    switchSettingsTab(tab === 'permissions' ? 'tools' : 'skills');
}

/**
 * Append `?session_id=` (or `&session_id=`) so workspace-scoped endpoints such as
 * `/api/skills` resolve the session's active project instead of the global sandbox.
 */
function sessionScopedUrl(url) {
    if (!currentSessionId) return url;
    const sep = url.includes('?') ? '&' : '?';
    return `${url}${sep}session_id=${encodeURIComponent(currentSessionId)}`;
}

async function fetchSkills() {
    try {
        let url = sessionScopedUrl('/api/skills');
        const res = await fetch(url);
        if (!res.ok) return;
        allSkills = await res.json();
        renderSkillsList();
    } catch (_) {}
}

function renderSkillsList() {
    if (!skillsListContainer) return;
    if (!allSkills || allSkills.length === 0) {
        skillsListContainer.innerHTML = '<div class="inspector-empty">No skills found in <code>.harness/skills/</code>. Click "+ New Skill" to create a reusable <code>/command</code>.</div>';
        return;
    }

    skillsListContainer.innerHTML = '';
    allSkills.forEach(skill => {
        const card = document.createElement('div');
        card.className = 'skill-card';
        const isEnabled = skill.enabled !== false;
        card.innerHTML = `
            <div class="skill-card-meta">
                <div class="skill-card-title-row">
                    <span class="skill-slash-badge"><span class="skill-slash-icon">${getCommandSvgIcon('skill', 13)}</span><span>/${escapeHtml(skill.name)}</span></span>
                    <span class="${isEnabled ? 'badge-active' : 'badge-idle'}">${isEnabled ? 'Enabled' : 'Disabled'}</span>
                </div>
                <div class="skill-card-desc">${escapeHtml(skill.description || 'Custom workspace skill')}</div>
                <div class="skill-card-path">${escapeHtml(skill.path || '')}</div>
            </div>
            <div class="skill-card-actions">
                <button type="button" class="btn-card-action" data-toggle-skill="${escapeHtml(skill.name)}">${isEnabled ? 'Disable' : 'Enable'}</button>
                <button type="button" class="btn-card-action primary" data-edit-skill="${escapeHtml(skill.name)}">Edit</button>
                <button type="button" class="btn-card-action danger" data-del-skill="${escapeHtml(skill.name)}">Delete</button>
            </div>
        `;

        card.querySelector('[data-toggle-skill]').addEventListener('click', () => toggleSkillEnabled(skill));
        card.querySelector('[data-edit-skill]').addEventListener('click', () => showSkillEditor(skill));
        card.querySelector('[data-del-skill]').addEventListener('click', () => deleteSkill(skill.name));
        skillsListContainer.appendChild(card);
    });
}

function showSkillEditor(skill = null) {
    if (!skillEditorCard) return;
    skillEditorCard.classList.remove('hidden');
    if (skill) {
        if (skillEditorTitle) skillEditorTitle.textContent = `Edit Skill: /${skill.name}`;
        if (editSkillName) editSkillName.value = skill.name || '';
        if (editSkillDesc) editSkillDesc.value = skill.description || '';
        if (editSkillContent) editSkillContent.value = skill.content || '';
        if (editSkillEnabled) editSkillEnabled.checked = skill.enabled !== false;
    } else {
        if (skillEditorTitle) skillEditorTitle.textContent = 'Create Skill';
        if (editSkillName) editSkillName.value = '';
        if (editSkillDesc) editSkillDesc.value = '';
        if (editSkillContent) editSkillContent.value = '';
        if (editSkillEnabled) editSkillEnabled.checked = true;
        if (editSkillName) editSkillName.focus();
    }
}

function hideSkillEditor() {
    if (skillEditorCard) skillEditorCard.classList.add('hidden');
}

async function saveSkill() {
    const name = editSkillName ? editSkillName.value.trim() : '';
    const description = editSkillDesc ? editSkillDesc.value.trim() : '';
    const content = editSkillContent ? editSkillContent.value : '';
    const enabled = editSkillEnabled ? editSkillEnabled.checked : true;

    if (!name) {
        showToast('Skill name is required.', 'warning');
        if (editSkillName) editSkillName.focus();
        return;
    }

    try {
        const res = await fetch(sessionScopedUrl('/api/skills'), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, description, content, enabled })
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            throw new Error(err.error || `HTTP ${res.status}`);
        }
        hideSkillEditor();
        await fetchSkills();
        showToast(`Saved skill "/${name}".`, 'success');
    } catch (e) {
        showToast('Failed to save skill: ' + e.message, 'error');
    }
}

async function toggleSkillEnabled(skill) {
    try {
        const res = await fetch(sessionScopedUrl('/api/skills'), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name: skill.name,
                description: skill.description || '',
                content: skill.content || '',
                enabled: skill.enabled === false
            })
        });
        if (res.ok) await fetchSkills();
    } catch (e) {
        showToast('Failed to toggle skill: ' + e.message, 'error');
    }
}

async function deleteSkill(name) {
    const ok = await uiConfirm(`Delete skill "/${name}"?`, {
        title: 'Delete Skill',
        confirmText: 'Delete Skill',
        danger: true
    });
    if (!ok) return;
    try {
        const res = await fetch(sessionScopedUrl(`/api/skills/${encodeURIComponent(name)}`), { method: 'DELETE' });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        await fetchSkills();
        showToast(`Deleted skill "/${name}".`, 'info');
    } catch (e) {
        showToast('Failed to delete skill: ' + e.message, 'error');
    }
}

// ==========================================================================
// Multi-Project Management
// ==========================================================================
let selectedProjectIcon = '📁';

async function fetchProjects() {
    try {
        const res = await fetch('/api/projects');
        if (!res.ok) return;
        allProjects = await res.json();

        const active = allProjects.find(p => p.is_active) || allProjects[0];
        if (active) {
            activeProjectId = active.id;
        }

        renderProjectsTree();
    } catch (e) {
        console.error('Failed to fetch projects', e);
    }
}

async function switchActiveProject(projectId) {
    try {
        const res = await fetch(`/api/projects/${encodeURIComponent(projectId)}/active`, {
            method: 'POST'
        });
        if (!res.ok) return;
        activeProjectId = projectId;
        activeComposerScope = 'project';
        await fetchProjects();
        await applyProjectDefaults(allProjects.find(p => p.id === projectId));
        await fetchSessions();
        await fetchSkills();
        startNewConversation(false);
        if (inspectorPane && !inspectorPane.classList.contains('collapsed')) {
            loadWorkspaceTree();
        }
    } catch (e) {
        console.error('Failed to switch active project', e);
    }
}

function renderProjectsTree() {
    renderProjectsList();
    renderConversationsList();
}

function renderConversationRowElement(s, isStandaloneKnowledge = false) {
    const item = document.createElement('div');
    const isActive = s.id === currentSessionId;
    item.className = `conv-row-item session-tree-item ${isActive ? 'active' : ''}`;
    item.setAttribute('data-session-id', s.id);
    const timeAgo = formatTimeAgo(s.updated_at || s.created_at);

    item.innerHTML = `
        <div class="conv-row-top">
            <span class="conv-row-title" title="${escapeHtml(s.task)}">${escapeHtml(s.task || 'Untitled Conversation')}</span>
            <span class="session-sidebar-time">${timeAgo}</span>
            <div class="conv-row-actions">
                <button type="button" class="conv-action-btn" data-conv-act="rename" title="Rename conversation">✎</button>
                <button type="button" class="conv-action-btn" data-conv-act="move" title="Move conversation">⇄</button>
                <button type="button" class="conv-action-btn danger" data-conv-act="delete" title="Delete conversation">✕</button>
            </div>
        </div>
    `;

    item.addEventListener('click', (e) => {
        if (e.target.closest('[data-conv-act]')) return;
        e.stopPropagation();
        const knownProjectIds = new Set(allProjects.map(p => p.id));
        if (!isStandaloneKnowledge && s.project_id && knownProjectIds.has(s.project_id)) {
            activeProjectId = s.project_id;
            activeComposerScope = 'project';
            renderProjectsList();
        } else {
            activeComposerScope = 'conversation';
        }
        document.querySelectorAll('.session-tree-item').forEach(el => el.classList.remove('active'));
        item.classList.add('active');
        loadHistoricalSession(s.id, s.task);
    });

    item.querySelectorAll('[data-conv-act]').forEach(btn => {
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            const act = btn.getAttribute('data-conv-act');
            if (act === 'rename') {
                await renameConversationById(s.id, s.task);
            } else if (act === 'move') {
                openMoveConvPopover(s.id, btn);
            } else if (act === 'delete') {
                await deleteConversationById(s.id, s.task);
            }
        });
    });

    return item;
}

function renderProjectsList() {
    if (!sidebarProjectsList) return;
    sidebarProjectsList.innerHTML = '';

    if (allProjects.length === 0) {
        sidebarProjectsList.innerHTML = '<div style="padding: 8px; color: var(--text-dim); font-size: 0.78rem;">No projects found</div>';
        return;
    }

    allProjects.forEach(project => {
        const isProjectActive = project.id === activeProjectId && activeComposerScope === 'project';
        const projectSessions = allSessions.filter(s => s.project_id === project.id);

        const projectWrap = document.createElement('div');
        projectWrap.className = 'project-tree-item';
        projectWrap.setAttribute('data-project-id', project.id);

        const folderRow = document.createElement('div');
        folderRow.className = `project-folder-row ${isProjectActive ? 'active' : ''}`;
        folderRow.title = `Project: ${project.name} (${project.path})`;

        folderRow.innerHTML = `
            <span class="project-tree-arrow">▾</span>
            <span class="project-tree-icon">${project.icon || '📁'}</span>
            <span class="project-tree-name">${escapeHtml(project.name)}</span>
            ${projectSessions.length > 0 ? `<span class="project-tree-badge">${projectSessions.length}</span>` : ''}
            <div class="project-tree-actions">
                <button type="button" class="btn-tree-action btn-proj-settings" title="Project Default Settings">⚙</button>
                <button type="button" class="btn-tree-action btn-add-session-in-proj" title="New Conversation in ${escapeHtml(project.name)}">+</button>
                ${allProjects.length > 1 ? `<button type="button" class="btn-tree-action btn-del-proj" title="Delete project">✕</button>` : ''}
            </div>
        `;

        folderRow.addEventListener('click', (e) => {
            if (e.target.closest('.btn-tree-action')) return;
            if (e.target.classList.contains('project-tree-arrow')) {
                projectWrap.classList.toggle('collapsed');
                return;
            }
            if (project.id !== activeProjectId || activeComposerScope !== 'project') {
                switchActiveProject(project.id);
            } else {
                projectWrap.classList.toggle('collapsed');
            }
        });

        // Right-click a project for settings, new conversation, copy path, or delete.
        folderRow.addEventListener('contextmenu', (ev) => {
            ev.preventDefault();
            openProjectContextMenu(ev, project);
        });

        const btnSettings = folderRow.querySelector('.btn-proj-settings');
        if (btnSettings) {
            btnSettings.addEventListener('click', (e) => {
                e.stopPropagation();
                openProjectSettingsModal(project.id);
            });
        }

        const btnAdd = folderRow.querySelector('.btn-add-session-in-proj');
        if (btnAdd) {
            btnAdd.addEventListener('click', (e) => {
                e.stopPropagation();
                createSavedConversation(project.id);
            });
        }

        const btnDel = folderRow.querySelector('.btn-del-proj');
        if (btnDel) {
            btnDel.addEventListener('click', async (e) => {
                e.stopPropagation();
                const ok = await uiConfirm(
                    `Delete project "${project.name}"? Any conversations inside it will be moved to the active workspace.`,
                    { title: 'Delete Project', confirmText: 'Delete Project', danger: true }
                );
                if (ok) {
                    await deleteProject(project.id);
                }
            });
        }

        const sublist = document.createElement('div');
        sublist.className = 'project-sessions-sublist';
        projectSessions.forEach(s => {
            sublist.appendChild(renderConversationRowElement(s, false));
        });

        projectWrap.appendChild(folderRow);
        projectWrap.appendChild(sublist);
        sidebarProjectsList.appendChild(projectWrap);
    });
}

function renderConversationsList() {
    if (!sidebarConversationsList) return;
    sidebarConversationsList.innerHTML = '';

    const knownProjectIds = new Set(allProjects.map(p => p.id));
    const standalone = allSessions.filter(s =>
        s.project_id === CONVERSATIONS_PROJECT_ID || !s.project_id || !knownProjectIds.has(s.project_id)
    );

    if (standalone.length === 0) {
        sidebarConversationsList.innerHTML = `<div style="padding: 10px 8px; color: var(--text-dim); font-size: 0.75rem;">No standalone conversations yet. Click + for a knowledge-work chat.</div>`;
        return;
    }

    standalone.forEach(s => {
        sidebarConversationsList.appendChild(renderConversationRowElement(s, true));
    });
}

async function createSavedConversation(projectIdOverride = null) {
    const pid = projectIdOverride || CONVERSATIONS_PROJECT_ID;
    activeComposerScope = pid === CONVERSATIONS_PROJECT_ID ? 'conversation' : 'project';
    if (activeComposerScope === 'project') {
        activeProjectId = pid;
        await applyProjectDefaults(allProjects.find(p => p.id === pid));
    }
    const serverId = (currentConfig && currentConfig.active_server_id) || 'local';
    const orchMode = Boolean(currentConfig && currentConfig.orchestrator && currentConfig.orchestrator.enabled);
    try {
        const res = await fetch('/api/sessions/create', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                title: 'New Conversation',
                project_id: pid,
                server_id: serverId,
                orchestrator_mode: orchMode,
                model: (currentConfig && currentConfig.model) || '',
                provider_id: (currentConfig && currentConfig.provider) || '',
                reasoning_effort: (currentConfig && currentConfig.reasoning_effort) || ''
            })
        });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();
        const createdId = data.id || data.session_id;
        await fetchSessions();
        await fetchProjects();
        await loadHistoricalSession(createdId, 'New Conversation');
        if (taskInput) taskInput.focus();
    } catch (e) {
        showToast('Failed to create conversation: ' + e.message, 'error');
    }
}

async function renameConversationById(sessionId, currentTitle) {
    const next = await uiPrompt('Rename conversation:', {
        title: 'Rename Conversation',
        defaultValue: currentTitle || '',
        confirmText: 'Save Name'
    });
    if (!next || !next.trim()) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}`, {
            method: 'PATCH',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ title: next.trim() })
        });
        if (res.ok) {
            if (currentSessionId === sessionId) updateBreadcrumbSessionUI(next.trim());
            await fetchSessions();
        }
    } catch (e) {
        showToast('Rename failed: ' + e.message, 'error');
    }
}

async function deleteConversationById(sessionId, title) {
    const ok = await uiConfirm(`Delete conversation "${title || 'Untitled'}" permanently?`, {
        title: 'Delete Conversation',
        confirmText: 'Delete',
        danger: true
    });
    if (!ok) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}`, { method: 'DELETE' });
        if (res.ok) {
            if (currentSessionId === sessionId) startNewConversation();
            await fetchSessions();
            await fetchProjects();
            showToast('Conversation deleted.', 'info');
        }
    } catch (e) {
        showToast('Delete failed: ' + e.message, 'error');
    }
}

function openMoveConvPopover(sessionId, anchorEl) {
    if (!moveConvPopover || !moveConvList) return;
    pendingMoveConvId = sessionId;
    closeAllPopovers();
    moveConvList.innerHTML = '';
    const sess = allSessions.find(s => s.id === sessionId);
    const targets = [
        { id: CONVERSATIONS_PROJECT_ID, name: 'Conversations (Knowledge Only)', icon: '💬' },
        ...allProjects
    ];
    targets.forEach(p => {
        const isCurrent = sess && (sess.project_id === p.id || (p.id === CONVERSATIONS_PROJECT_ID && !sess.project_id));
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `popover-item${isCurrent ? ' active' : ''}`;
        btn.innerHTML = `<span class="popover-item-title">${escapeHtml(p.icon || '📁')} ${escapeHtml(p.name)}</span>${isCurrent ? '<span class="popover-item-check">✓</span>' : ''}`;
        btn.addEventListener('click', async () => {
            moveConvPopover.classList.add('hidden');
            if (isCurrent) return;
            try {
                const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}`, {
                    method: 'PATCH',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ project_id: p.id })
                });
                if (res.ok) {
                    await fetchSessions();
                    await fetchProjects();
                    showToast(`Moved conversation to ${p.name}.`, 'success');
                }
            } catch (e) {
                showToast('Move failed: ' + e.message, 'error');
            }
        });
        moveConvList.appendChild(btn);
    });
    const rect = anchorEl.getBoundingClientRect();
    moveConvPopover.style.left = `${Math.min(window.innerWidth - 260, Math.max(8, rect.left))}px`;
    moveConvPopover.style.top = `${Math.min(window.innerHeight - 240, rect.bottom + 6)}px`;
    moveConvPopover.classList.remove('hidden');
}

function autoFillProjectNameFromPath(nameInputEl, dirPath) {
    if (!nameInputEl || nameInputEl.value.trim() || !dirPath) return;
    const segments = String(dirPath).split(/[\\/]/).filter(Boolean);
    if (segments.length > 0) {
        const last = segments[segments.length - 1];
        if (/^[A-Za-z]:$/.test(last)) return;
        nameInputEl.value = last.replace(/[-_]/g, ' ').replace(/\b\w/g, l => l.toUpperCase());
    }
}

// ==========================================================================
// In-Browser Sidebar Inline Project Creator
// ==========================================================================
let selectedInlineIcon = '📁';
let currentInlineBrowsePath = '';
let parentInlineBrowsePath = null;

function toggleInlineProjectForm(forceOpen) {
    if (!sidebarInlineProjectForm) return;
    const isCurrentlyHidden = sidebarInlineProjectForm.classList.contains('hidden');
    const shouldOpen = forceOpen !== undefined ? forceOpen : isCurrentlyHidden;

    if (shouldOpen) {
        closeAllPopovers();
        sidebarInlineProjectForm.classList.remove('hidden');
        if (inlineProjectName) {
            inlineProjectName.value = '';
            inlineProjectName.focus();
        }
        if (inlineProjectPath) {
            inlineProjectPath.value = '';
        }
    } else {
        sidebarInlineProjectForm.classList.add('hidden');
        if (inlineBrowserDrawer) inlineBrowserDrawer.classList.add('hidden');
        if (btnToggleInlineTree) btnToggleInlineTree.textContent = '▾';
    }
}

async function fetchInlineDirectory(path) {
    if (!inlineBrowserTreeList) return;
    inlineBrowserTreeList.innerHTML = '<div style="padding: 6px 8px; color: var(--text-dim); font-size: 0.76rem;">Loading folders...</div>';

    try {
        const url = `/api/filesystem/browse${path ? `?path=${encodeURIComponent(path)}` : ''}`;
        const res = await fetch(url);
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            const msg = err.error || `Failed to list folders (HTTP ${res.status})`;
            inlineBrowserTreeList.innerHTML = `<div style="padding: 6px 8px; color: var(--accent-red); font-size: 0.76rem;">${escapeHtml(msg)}</div>`;
            return;
        }

        const data = await res.json();
        currentInlineBrowsePath = data.current_path;
        parentInlineBrowsePath = data.parent_path;

        if (inlineBrowserCurrentLabel) {
            const display = data.current_path.length > 28 ? '...' + data.current_path.slice(-25) : data.current_path;
            inlineBrowserCurrentLabel.textContent = display;
            inlineBrowserCurrentLabel.title = data.current_path;
        }
        if (btnInlineBrowserUp) {
            btnInlineBrowserUp.disabled = !parentInlineBrowsePath;
        }

        if (inlineBrowserShortcuts) {
            inlineBrowserShortcuts.innerHTML = '';
            const chips = [];
            (data.favorites || []).slice(0, 3).forEach(f => {
                chips.push({ label: `${f.icon || '📁'} ${f.name}`, path: f.path });
            });
            (data.drives || []).forEach(d => {
                chips.push({ label: d.replace(/\\$/, ''), path: d });
            });
            chips.forEach(c => {
                const btn = document.createElement('button');
                btn.type = 'button';
                btn.className = 'inline-shortcut-chip';
                if (c.path.toLowerCase() === currentInlineBrowsePath.toLowerCase()) {
                    btn.classList.add('active');
                }
                btn.textContent = c.label;
                btn.title = c.path;
                btn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    fetchInlineDirectory(c.path);
                });
                inlineBrowserShortcuts.appendChild(btn);
            });
        }

        inlineBrowserTreeList.innerHTML = '';
        const dirs = (data.entries || []).filter(e => e.is_dir);
        if (dirs.length === 0) {
            inlineBrowserTreeList.innerHTML = '<div style="padding: 6px 8px; color: var(--text-dim); font-size: 0.74rem;">(No subfolders — click "✓ Use" above or "+ Folder")</div>';
            return;
        }

        dirs.forEach(entry => {
            const item = document.createElement('div');
            item.className = 'inline-browser-item';
            if (inlineProjectPath && inlineProjectPath.value.toLowerCase() === entry.path.toLowerCase()) {
                item.classList.add('selected');
            }
            item.innerHTML = `
                <span class="inline-item-icon">📁</span>
                <span class="inline-item-name">${escapeHtml(entry.name)}</span>
                <button type="button" class="inline-item-open-btn" title="Open folder">›</button>
            `;
            item.title = `${entry.path}\nClick to select • Double-click or click › to open`;

            const openBtn = item.querySelector('.inline-item-open-btn');
            if (openBtn) {
                openBtn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    fetchInlineDirectory(entry.path);
                });
            }

            item.addEventListener('click', (e) => {
                e.stopPropagation();
                if (inlineProjectPath) inlineProjectPath.value = entry.path;
                autoFillProjectNameFromPath(inlineProjectName, entry.path);
                document.querySelectorAll('#inline-browser-tree-list .inline-browser-item').forEach(el => el.classList.remove('selected'));
                item.classList.add('selected');
            });

            item.addEventListener('dblclick', (e) => {
                e.stopPropagation();
                fetchInlineDirectory(entry.path);
            });

            inlineBrowserTreeList.appendChild(item);
        });
    } catch (e) {
        inlineBrowserTreeList.innerHTML = `<div style="padding: 6px 8px; color: var(--accent-red); font-size: 0.76rem;">${escapeHtml(e.message)}</div>`;
    }
}

async function createFolderInInlineBrowser() {
    if (!currentInlineBrowsePath) {
        await fetchInlineDirectory('');
        if (!currentInlineBrowsePath) {
            showToast('Open a valid parent directory first.', 'warning');
            return;
        }
    }
    const folderName = await uiPrompt('Enter new folder name:', {
        title: 'Create Folder',
        placeholder: 'e.g. my-project',
        confirmText: 'Create Folder'
    });
    if (!folderName || !folderName.trim()) return;

    try {
        const res = await fetch('/api/filesystem/mkdir', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                parent_path: currentInlineBrowsePath,
                name: folderName.trim()
            })
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            showToast(err.error || 'Failed to create directory', 'error');
            return;
        }
        const data = await res.json();
        if (inlineProjectPath) inlineProjectPath.value = data.path;
        autoFillProjectNameFromPath(inlineProjectName, data.path);
        await fetchInlineDirectory(currentInlineBrowsePath);
        showToast(`Created folder '${folderName.trim()}'.`, 'success');
    } catch (e) {
        showToast('Error creating folder: ' + e.message, 'error');
    }
}

async function submitInlineProject() {
    const name = inlineProjectName ? inlineProjectName.value.trim() : '';
    let path = inlineProjectPath ? inlineProjectPath.value.trim() : '';

    if (!name) {
        if (inlineProjectName) inlineProjectName.focus();
        showToast('Please enter a project name.', 'warning');
        return;
    }

    if (!path) {
        const sanitized = name.toLowerCase().replace(/[^a-z0-9_-]/g, '-');
        path = `projects\\${sanitized}`;
    }

    if (btnSubmitInlineProject) btnSubmitInlineProject.disabled = true;
    try {
        const res = await fetch('/api/projects', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name,
                path,
                icon: selectedInlineIcon || '📁',
                set_active: true
            })
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            throw new Error(err.error || 'Failed to create project');
        }
        toggleInlineProjectForm(false);
        await fetchProjects();
        await fetchSessions();
        await fetchSkills();
        startNewConversation();
        showToast(`Created project "${name}".`, 'success');
    } catch (err) {
        showToast('Error: ' + err.message, 'error');
    } finally {
        if (btnSubmitInlineProject) btnSubmitInlineProject.disabled = false;
    }
}

function openNewProjectModal() {
    closeAllPopovers();
    if (projectNameInput) projectNameInput.value = '';
    if (projectPathInput) {
        projectPathInput.value = '';
        projectPathInput.placeholder = `projects\\workspace-${Date.now().toString().slice(-4)}`;
    }
    if (projectSetActiveCheck) projectSetActiveCheck.checked = true;
    selectedProjectIcon = '📁';
    document.querySelectorAll('#project-icon-picker .icon-choice').forEach(btn => {
        btn.classList.toggle('active', btn.getAttribute('data-icon') === '📁');
    });
    projectModal.classList.remove('hidden');
    if (projectNameInput) projectNameInput.focus();
}

function closeNewProjectModal() {
    if (projectModal) projectModal.classList.add('hidden');
}

async function submitNewProject() {
    const name = projectNameInput.value.trim();
    let path = projectPathInput.value.trim();
    if (!name) {
        showToast('Please enter a project name.', 'warning');
        projectNameInput.focus();
        return;
    }
    if (!path) {
        const sanitized = name.toLowerCase().replace(/[^a-z0-9_-]/g, '-');
        path = `projects\\${sanitized}`;
    }

    btnSubmitProject.disabled = true;
    try {
        const res = await fetch('/api/projects', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name,
                path,
                icon: selectedProjectIcon,
                set_active: projectSetActiveCheck.checked,
                extra_paths: (document.getElementById('project-extra-paths')?.value || '')
                    .split('\n')
                    .map(s => s.trim())
                    .filter(Boolean)
            })
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            throw new Error(err.error || 'Failed to save project');
        }
        closeNewProjectModal();
        await fetchProjects();
        await fetchSessions();
        await fetchSkills();
        if (projectSetActiveCheck.checked) startNewConversation();
        showToast(`Created project "${name}".`, 'success');
    } catch (err) {
        showToast('Error: ' + err.message, 'error');
    } finally {
        btnSubmitProject.disabled = false;
    }
}

async function deleteProject(projectId) {
    try {
        const res = await fetch(`/api/projects/${encodeURIComponent(projectId)}`, { method: 'DELETE' });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            showToast(err.error || 'Failed to delete project', 'error');
            return;
        }
        const data = await res.json().catch(() => ({}));
        await fetchProjects();
        await fetchSessions();
        const moved = data.reassigned_sessions || 0;
        showToast(
            moved > 0
                ? `Project deleted (${moved} conversation${moved === 1 ? '' : 's'} moved to workspace).`
                : 'Project deleted.',
            'info'
        );
    } catch (e) {
        console.error('Failed to delete project', e);
        showToast('Failed to delete project: ' + e.message, 'error');
    }
}

// ==========================================================================
// In-Browser File & Folder Browser Modal
// ==========================================================================
let currentBrowsePath = '';
let selectedBrowsePath = '';
let parentBrowsePath = null;
let browseHistory = [];
let lastBrowseData = null;
let fileBrowserConfirmCallback = null;

async function openFileBrowser(initialPath, options = {}) {
    if (!fileBrowserModal) return;
    closeAllPopovers();
    const titleEl = document.getElementById('file-browser-title');
    if (titleEl) titleEl.textContent = options.title || 'Select Project Directory';
    fileBrowserConfirmCallback = typeof options.onSelect === 'function' ? options.onSelect : null;
    browseHistory = [];
    selectedBrowsePath = '';
    if (browserSelectedLabel) browserSelectedLabel.textContent = '--';
    fileBrowserModal.classList.remove('hidden');
    await fetchDirectory(initialPath || '', { strict: false, pushHistory: false });
}

function closeFileBrowser() {
    if (fileBrowserModal) fileBrowserModal.classList.add('hidden');
    fileBrowserConfirmCallback = null;
}

function renderBrowserSidebarShortcuts(data) {
    if (!data) return;
    if (browserFavoritesList) {
        browserFavoritesList.innerHTML = '';
        (data.favorites || []).forEach(fav => {
            const item = document.createElement('div');
            const isCurrent = fav.path.toLowerCase() === currentBrowsePath.toLowerCase();
            item.className = `browser-shortcut-item ${isCurrent ? 'active' : ''}`;
            item.innerHTML = `<span>${fav.icon}</span><span>${escapeHtml(fav.name)}</span>`;
            item.title = fav.path;
            item.addEventListener('click', () => {
                fetchDirectory(fav.path, { pushHistory: true });
            });
            browserFavoritesList.appendChild(item);
        });
    }

    if (browserDrivesList) {
        browserDrivesList.innerHTML = '';
        (data.drives || []).forEach(drive => {
            const item = document.createElement('div');
            const isCurrent = currentBrowsePath.toLowerCase().startsWith(drive.toLowerCase());
            item.className = `browser-shortcut-item ${isCurrent ? 'active' : ''}`;
            item.innerHTML = `<span>💾</span><span>${escapeHtml(drive)}</span>`;
            item.addEventListener('click', () => {
                fetchDirectory(drive, { pushHistory: true });
            });
            browserDrivesList.appendChild(item);
        });
    }
}

function renderBrowserEntries(data) {
    if (!browserEntriesList || !data) return;
    browserEntriesList.innerHTML = '';

    if (data.notice) {
        const noticeEl = document.createElement('div');
        noticeEl.className = 'browser-notice-banner';
        noticeEl.textContent = data.notice;
        browserEntriesList.appendChild(noticeEl);
    }

    const dirsOnly = browserDirsOnlyToggle ? browserDirsOnlyToggle.checked : true;
    const rawEntries = data.entries || [];
    const entries = dirsOnly ? rawEntries.filter(e => e.is_dir) : rawEntries;

    if (entries.length === 0) {
        const emptyEl = document.createElement('div');
        emptyEl.style.cssText = 'padding: 24px; text-align: center; color: var(--text-dim); font-size: 0.85rem;';
        emptyEl.textContent = dirsOnly
            ? '(No subfolders in this directory — click "Select Folder" to choose this folder or "+ New Folder")'
            : '(Empty folder)';
        browserEntriesList.appendChild(emptyEl);
        return;
    }

    entries.forEach(entry => {
        const row = document.createElement('div');
        const isSelected = entry.is_dir && entry.path === selectedBrowsePath;
        row.className = `browser-entry-row ${isSelected ? 'selected' : ''} ${entry.is_dir ? '' : 'disabled-file'}`;
        row.dataset.path = entry.path;
        const icon = entry.is_dir ? '📁' : '📄';
        const sizeStr = entry.is_dir ? '--' : formatBytes(entry.size_bytes || 0);
        const dateStr = entry.modified ? formatTimeAgo(entry.modified) : '--';

        row.innerHTML = `
            <span class="col-name"><span>${icon}</span><span class="entry-title">${escapeHtml(entry.name)}</span>${entry.is_dir ? '<button type="button" class="browser-entry-open-btn" title="Open folder">Open</button>' : ''}</span>
            <span class="col-type">${entry.is_dir ? 'Folder' : 'File'}</span>
            <span class="col-size">${sizeStr}</span>
            <span class="col-date">${dateStr}</span>
        `;

        if (entry.is_dir) {
            const openBtn = row.querySelector('.browser-entry-open-btn');
            if (openBtn) {
                openBtn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    fetchDirectory(entry.path, { pushHistory: true });
                });
            }

            row.addEventListener('click', () => {
                selectedBrowsePath = entry.path;
                document.querySelectorAll('.browser-entry-row').forEach(r => r.classList.remove('selected'));
                row.classList.add('selected');
                if (browserSelectedLabel) browserSelectedLabel.textContent = entry.path;
            });

            row.addEventListener('dblclick', () => {
                fetchDirectory(entry.path, { pushHistory: true });
            });
        }

        browserEntriesList.appendChild(row);

        // Right-click for a context menu (open, use, rename, delete, copy path).
        row.addEventListener('contextmenu', (ev) => showBrowserContextMenu(ev, entry));
    });
}

async function fetchDirectory(path, opts = {}) {
    if (!browserEntriesList) return;
    const prevPath = currentBrowsePath;
    browserEntriesList.innerHTML = '<div class="loading-state">Loading directory...</div>';

    try {
        const params = new URLSearchParams();
        if (path) params.set('path', path);
        if (opts.strict) params.set('strict', 'true');
        const qs = params.toString();
        const url = `/api/filesystem/browse${qs ? `?${qs}` : ''}`;
        const res = await fetch(url);
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            const msg = err.error || `Failed to read directory (HTTP ${res.status})`;
            browserEntriesList.innerHTML = `<div style="padding: 16px; color: var(--accent-red); font-size: 0.85rem;">${escapeHtml(msg)}</div>`;
            if (!lastBrowseData) {
                const fbRes = await fetch('/api/filesystem/browse').catch(() => null);
                if (fbRes && fbRes.ok) {
                    const fbData = await fbRes.json();
                    renderBrowserSidebarShortcuts(fbData);
                }
            }
            return;
        }

        const data = await res.json();
        if (opts.pushHistory && prevPath && prevPath !== data.current_path) {
            browseHistory.push(prevPath);
        }
        lastBrowseData = data;
        currentBrowsePath = data.current_path;
        parentBrowsePath = data.parent_path;
        selectedBrowsePath = '';

        if (browserPathInput) browserPathInput.value = currentBrowsePath;
        if (browserSelectedLabel) browserSelectedLabel.textContent = currentBrowsePath;
        if (btnBrowserBack) btnBrowserBack.disabled = browseHistory.length === 0;
        if (btnBrowserUp) btnBrowserUp.disabled = !parentBrowsePath;

        renderBrowserSidebarShortcuts(data);
        renderBrowserEntries(data);
    } catch (e) {
        browserEntriesList.innerHTML = `<div style="padding: 16px; color: var(--accent-red); font-size: 0.85rem;">Error: ${escapeHtml(e.message)}</div>`;
    }
}

function goBrowserBack() {
    if (browseHistory.length > 0) {
        fetchDirectory(browseHistory.pop(), { pushHistory: false });
    }
}

function goBrowserUp() {
    if (parentBrowsePath) {
        fetchDirectory(parentBrowsePath, { pushHistory: true });
    }
}

function refreshBrowser() {
    fetchDirectory(currentBrowsePath, { pushHistory: false });
}

async function createDirectoryInBrowser() {
    if (!currentBrowsePath) {
        showToast('Browse to a valid directory first.', 'warning');
        return;
    }
    const folderName = await uiPrompt('Enter new folder name:', {
        title: 'Create Folder',
        placeholder: 'e.g. new-workspace',
        confirmText: 'Create Folder'
    });
    if (!folderName || !folderName.trim()) return;

    try {
        const res = await fetch('/api/filesystem/mkdir', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                parent_path: currentBrowsePath,
                name: folderName.trim()
            })
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            showToast(err.error || 'Failed to create folder', 'error');
            return;
        }
        const data = await res.json();
        await fetchDirectory(currentBrowsePath, { pushHistory: false });
        selectedBrowsePath = data.path;
        if (browserSelectedLabel) browserSelectedLabel.textContent = data.path;
        document.querySelectorAll('.browser-entry-row').forEach(r => {
            const titleEl = r.querySelector('.entry-title');
            if (titleEl && titleEl.textContent === folderName.trim()) {
                r.classList.add('selected');
            }
        });
        showToast(`Created folder '${folderName.trim()}'.`, 'success');
    } catch (e) {
        showToast('Error creating folder: ' + e.message, 'error');
    }
}

function confirmBrowserSelection() {
    const chosen = selectedBrowsePath || currentBrowsePath;
    if (!chosen) {
        showToast('Please select a directory first.', 'warning');
        return;
    }
    if (fileBrowserConfirmCallback) {
        fileBrowserConfirmCallback(chosen);
    } else {
        if (projectPathInput) projectPathInput.value = chosen;
        autoFillProjectNameFromPath(projectNameInput, chosen);
    }
    closeFileBrowser();
}

function formatBytes(bytes) {
    if (!bytes) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

// ==========================================================================
// Session Execution & Stop Controls
// ==========================================================================
function setSessionRunning(isRunning) {
    isMainSessionRunning = !!isRunning;
    updateTaskRunningIndicator();
}

async function stopActiveSession() {
    if (!currentSessionId) return;
    let ok = false;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/stop`, { method: 'POST' });
        ok = res.ok;
    } catch (e) {
        console.error('Failed to stop session', e);
    }
    if (ok) {
        setSessionRunning(false);
        fetchSessionSwarmState(currentSessionId);
        return;
    }
    // The stop did NOT take effect. Leaving the green "running" pill up would
    // tell the user the run ended while the agent keeps working, so drop it and
    // say why instead of only logging to the console.
    setSessionRunning(false);
    finalizeOrphanedStreamStates();
    showToast(
        'Could not stop this conversation (the server did not confirm it). ' +
        'It may still be running - try Stop again, or check the server is reachable.',
        'error', 9000
    );
}

async function fetchSessions() {
    try {
        const res = await fetch('/api/sessions');
        if (!res.ok) return;
        allSessions = await res.json();
        renderProjectsTree();
    } catch (_) {}
}

function formatTimeAgo(isoString) {
    if (!isoString) return 'just now';
    const date = new Date(isoString);
    const diffSec = Math.floor((new Date() - date) / 1000);
    if (diffSec < 60) return 'just now';
    if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m`;
    if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h`;
    return `${Math.floor(diffSec / 86400)}d`;
}

function startNewConversation(knowledgeMode = false) {
    if (activeEventSource) {
        activeEventSource.close();
        activeEventSource = null;
    }
    activeEventStreamSessionId = null;
    setSessionRunning(false);
    currentSessionId = null;
    activeComposerScope = knowledgeMode ? 'conversation' : 'project';
    activeReasoningDrawer = null;
    activeMessageBox = null;
    activeTurnId = null;
    queuedPrompts = [];
    renderQueuedPromptsTray();
    clearComposerSlashCommand();
    clearStagedAttachments();
    syncShouldAutoScrollFlag(true);
    resetInspectorState();

    if (activeComposerScope === 'conversation') {
        updateBreadcrumbSessionUI('New Conversation');
    } else {
        updateBreadcrumbSessionUI('New Project Chat');
    }
    renderProjectsList();
    streamContainer.innerHTML = '';
    // Any outstanding approval belonged to the stream we just discarded.
    clearAllPendingPermissions();
    if (welcomeCanvas) {
        welcomeCanvas.style.display = 'flex';
        streamContainer.appendChild(welcomeCanvas);
    }
    document.querySelectorAll('.session-tree-item').forEach(el => el.classList.remove('active'));
    taskInput.value = '';
    autoResizeTextarea();
    taskInput.focus();

    lastCurrentContextTokens = 0;
    const maxCtx = currentConfig?.memory?.max_context_tokens || 128000;
    applyResolvedContextWindow(maxCtx);
    if (analyticsCtxProgress) {
        analyticsCtxProgress.style.width = '0%';
        analyticsCtxProgress.style.background = 'linear-gradient(90deg, var(--accent-blue), #60a5fa)';
    }
    if (analyticsPromptTokens) analyticsPromptTokens.textContent = '0';
    if (analyticsCompletionTokens) analyticsCompletionTokens.textContent = '0';
    if (analyticsCachedTokens) analyticsCachedTokens.textContent = '0';
    if (analyticsCacheHitRate) analyticsCacheHitRate.textContent = '0.0%';
    if (analyticsTotalTokens) analyticsTotalTokens.textContent = '0';
    if (analyticsCostUsd) analyticsCostUsd.textContent = '$0.0000';
}

function setComposerSlashCommand(commandName, meta = null) {
    const cleanCmd = String(commandName || '').replace(/^\/+/, '').trim().toLowerCase();
    if (!cleanCmd) {
        clearComposerSlashCommand();
        return;
    }
    const found = meta || getSlashOptions().find(o => o.name.slice(1).toLowerCase() === cleanCmd);
    const iconKey = found?.iconKey || cleanCmd;
    const label = found?.label || found?.desc || `/${cleanCmd} directive`;
    selectedSlashCommand = { command: cleanCmd, iconKey, label };

    if (composerSlashIcon) composerSlashIcon.innerHTML = getCommandSvgIcon(iconKey, 13);
    if (composerSlashCmd) composerSlashCmd.textContent = `/${cleanCmd}`;
    if (composerSlashLabel) composerSlashLabel.textContent = label;
    if (composerSlashRow) composerSlashRow.classList.remove('hidden');
}

function clearComposerSlashCommand() {
    selectedSlashCommand = null;
    if (composerSlashRow) composerSlashRow.classList.add('hidden');
    if (composerSlashIcon) composerSlashIcon.innerHTML = '';
    if (composerSlashCmd) composerSlashCmd.textContent = '';
    if (composerSlashLabel) composerSlashLabel.textContent = '';
}

function parseSlashSubmission(rawTask) {
    const trimmed = (rawTask || '').trim();
    if (selectedSlashCommand) {
        return {
            slashCommand: selectedSlashCommand.command,
            slashArgs: trimmed,
            content: trimmed
        };
    }
    const match = trimmed.match(/^\/([a-zA-Z0-9_-]+)(?:\s+([\s\S]*))?$/);
    if (match) {
        const cmd = match[1].toLowerCase();
        const rest = (match[2] || '').trim();
        const known = cmd === 'clear' || getSlashOptions().some(o => o.name.slice(1).toLowerCase() === cmd);
        if (known) {
            return {
                slashCommand: cmd,
                slashArgs: rest,
                content: rest
            };
        }
    }
    return {
        slashCommand: null,
        slashArgs: null,
        content: trimmed
    };
}

function inferAttachmentKind(file) {
    const mime = ((file && file.type) || '').toLowerCase();
    const name = ((file && file.name) || '').toLowerCase();
    if (mime.startsWith('image/') || /\.(png|jpe?g|gif|webp|bmp|svg|ico)$/.test(name)) return 'image';
    if (mime.startsWith('video/') || /\.(mp4|webm|mov|m4v|avi|mkv)$/.test(name)) return 'video';
    if (mime.startsWith('audio/') || /\.(mp3|wav|ogg|m4a|flac|aac|opus)$/.test(name)) return 'audio';
    if (mime === 'application/pdf' || name.endsWith('.pdf')) return 'pdf';
    if (mime.startsWith('text/') || /\.(txt|md|markdown|json|jsonl|csv|tsv|rs|js|mjs|cjs|ts|tsx|jsx|py|html?|css|scss|toml|ya?ml|xml|sql|sh|bash|ps1|log|ini|conf|env|c|cpp|h|hpp|go|java|rb|php)$/.test(name)) return 'text';
    return 'file';
}

function readFileAsDataUrl(file) {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result || ''));
        reader.onerror = () => reject(new Error(`Failed to read ${file.name}`));
        reader.readAsDataURL(file);
    });
}

function readFileAsText(file) {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result || ''));
        reader.onerror = () => reject(new Error(`Failed to read ${file.name}`));
        reader.readAsText(file);
    });
}

function formatAttachmentBytes(bytes) {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function getAttachmentCompatibilityWarning(att, caps) {
    if (!att || !caps) return '';
    if (att.kind === 'image' && !caps.vision) return 'Active model lacks native vision — sent as file metadata note';
    if (att.kind === 'video' && !caps.video) return 'Active model lacks video input — sent as metadata note';
    if (att.kind === 'audio' && !caps.audio) return 'Active model lacks audio input — sent as metadata note';
    if (att.kind === 'pdf' && !caps.pdf && !caps.files) return 'Active model lacks native PDF upload — sent as metadata note';
    return '';
}

async function stageFiles(fileList) {
    const incoming = Array.from(fileList || []);
    if (incoming.length === 0) return;

    const maxCount = 8;
    const maxMediaBytes = 18 * 1024 * 1024;
    const maxTextBytes = 256 * 1024;

    for (const file of incoming) {
        if (stagedAttachments.length >= maxCount) {
            showToast(`Maximum ${maxCount} attachments per message reached.`, 'warning');
            break;
        }
        const kind = inferAttachmentKind(file);
        const mimeType = file.type || (kind === 'text' ? 'text/plain' : 'application/octet-stream');

        if (kind === 'text') {
            if (file.size > maxTextBytes) {
                const slice = file.slice(0, maxTextBytes);
                const partial = await readFileAsText(slice);
                stagedAttachments.push({
                    id: `att_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
                    name: file.name || 'attachment.txt',
                    mime_type: mimeType,
                    kind,
                    size_bytes: file.size,
                    text_content: `${partial}\n\n[... truncated to 256 KB from ${formatAttachmentBytes(file.size)} ...]`
                });
            } else {
                const textContent = await readFileAsText(file);
                stagedAttachments.push({
                    id: `att_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
                    name: file.name || 'attachment.txt',
                    mime_type: mimeType,
                    kind,
                    size_bytes: file.size,
                    text_content: textContent
                });
            }
        } else {
            if (file.size > maxMediaBytes) {
                showToast(`Skipped "${file.name}" (${formatAttachmentBytes(file.size)}): max 18 MB per binary file.`, 'warning');
                continue;
            }
            const dataUrl = await readFileAsDataUrl(file);
            stagedAttachments.push({
                id: `att_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
                name: file.name || 'attachment',
                mime_type: mimeType,
                kind,
                size_bytes: file.size,
                data_url: dataUrl
            });
        }
    }

    renderComposerAttachments();
    updateSubmitButtonState();
}

function clearStagedAttachments() {
    stagedAttachments = [];
    renderComposerAttachments();
}

function removeStagedAttachment(id) {
    stagedAttachments = stagedAttachments.filter(a => a.id !== id);
    renderComposerAttachments();
    updateSubmitButtonState();
}

function renderComposerAttachments() {
    if (!composerAttachmentsRow) return;
    if (!stagedAttachments || stagedAttachments.length === 0) {
        composerAttachmentsRow.innerHTML = '';
        composerAttachmentsRow.classList.add('hidden');
        return;
    }

    const activeId = (currentConfig && currentConfig.model) || '';
    const caps = getModelModalityCaps(activeId);
    composerAttachmentsRow.innerHTML = '';
    composerAttachmentsRow.classList.remove('hidden');

    stagedAttachments.forEach(att => {
        const chip = document.createElement('div');
        const warn = getAttachmentCompatibilityWarning(att, caps);
        chip.className = `composer-att-chip${warn ? ' has-warning' : ''}`;
        if (warn) chip.title = warn;

        let thumbHtml = '';
        if (att.kind === 'image' && att.data_url) {
            thumbHtml = `<img class="composer-att-thumb" src="${escapeHtml(att.data_url)}" alt="${escapeHtml(att.name)}">`;
        } else {
            const iconLabel = att.kind === 'video' ? 'VID' : att.kind === 'audio' ? 'AUD' : att.kind === 'pdf' ? 'PDF' : 'FILE';
            thumbHtml = `<span class="composer-att-badge">${iconLabel}</span>`;
        }

        chip.innerHTML = `
            ${thumbHtml}
            <div class="composer-att-meta">
                <span class="composer-att-name">${escapeHtml(att.name)}</span>
                <span class="composer-att-sub">${escapeHtml(formatAttachmentBytes(att.size_bytes))}${warn ? ` · <span class="att-warn-text">No native ${escapeHtml(att.kind)}</span>` : ''}</span>
            </div>
            <button type="button" class="composer-att-remove" title="Remove attachment">✕</button>
        `;
        const removeBtn = chip.querySelector('.composer-att-remove');
        if (removeBtn) {
            removeBtn.addEventListener('click', () => removeStagedAttachment(att.id));
        }
        composerAttachmentsRow.appendChild(chip);
    });
}

function renderAttachmentListHtml(attachments) {
    if (!Array.isArray(attachments) || attachments.length === 0) return '';
    const cards = attachments.map(att => {
        const name = escapeHtml(att.name || 'attachment');
        const size = escapeHtml(formatAttachmentBytes(Number(att.size_bytes) || 0));
        if (att.kind === 'image' && att.data_url) {
            return `<button type="button" class="user-att-img-card" data-lightbox-src="${escapeHtml(att.data_url)}" data-lightbox-title="${name}" title="${name} (${size})"><img src="${escapeHtml(att.data_url)}" alt="${name}"><span class="user-att-caption">${name} · ${size}</span></button>`;
        }
        if (att.kind === 'video' && att.data_url) {
            return `<div class="user-att-media-card"><video controls preload="metadata" src="${escapeHtml(att.data_url)}"></video><div class="user-att-caption">Video: ${name} · ${size}</div></div>`;
        }
        if (att.kind === 'audio' && att.data_url) {
            return `<div class="user-att-media-card audio"><audio controls preload="metadata" src="${escapeHtml(att.data_url)}"></audio><div class="user-att-caption">Audio: ${name} · ${size}</div></div>`;
        }
        const badge = att.kind === 'pdf' ? 'PDF' : att.kind === 'text' ? 'TEXT' : 'FILE';
        return `<div class="user-att-file-pill" title="${name} (${size})"><span class="composer-att-badge">${badge}</span><span class="user-att-file-name">${name}</span><span class="user-att-file-size">${size}</span></div>`;
    });
    return `<div class="user-attachments-grid">${cards.join('')}</div>`;
}

async function handlePromptSubmit() {
    const task = taskInput ? taskInput.value.trim() : '';
    if (!task && !selectedSlashCommand && stagedAttachments.length === 0) return;
    await submitPromptText(task, true);
}

async function submitPromptText(rawTask, clearInput = false) {
    const parsed = parseSlashSubmission(rawTask);
    const slashCommand = parsed.slashCommand;
    const slashArgs = parsed.slashArgs;
    const content = parsed.content;
    const attachmentsToSend = clearInput ? stagedAttachments.map(a => ({ ...a })) : [];

    if (!content && !slashCommand && attachmentsToSend.length === 0) return;

    if (slashCommand === 'clear' && !content && attachmentsToSend.length === 0) {
        startNewConversation();
        return;
    }

    if (isSubmittingPrompt) return;
    isSubmittingPrompt = true;

    try {
        scrollToBottom(true);

        if (currentSessionId) {
            if (clearInput) {
                taskInput.value = '';
                autoResizeTextarea();
                clearComposerSlashCommand();
                clearStagedAttachments();
            }
            if (welcomeCanvas) welcomeCanvas.style.display = 'none';

            await ensureEventStreamConnected(currentSessionId);

            if (!isMainSessionRunning) {
                setSessionRunning(true);
                activeReasoningDrawer = null;
                activeMessageBox = null;
                const optBubble = (!slashCommand && (content || attachmentsToSend.length > 0))
                    ? appendUserMessage(content, null, true, attachmentsToSend)
                    : null;

                try {
                    const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/message`, {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({
                            content,
                            attachments: attachmentsToSend,
                            slash_command: slashCommand,
                            slash_args: slashArgs,
                            mode: activeSendMode,
                            mock: mockModeEnabled
                        })
                    });
                    if (!res.ok) {
                        const errText = await res.text().catch(() => '');
                        throw new Error(errText || `HTTP ${res.status}`);
                    }
                    const data = await res.json();
                    if (data.status === 'running' && data.turn_id) {
                        activeTurnId = Number(data.turn_id);
                        if (optBubble) upgradeUserBubbleTurnId(optBubble, data.turn_id);
                    } else if (data.status === 'queued') {
                        if (optBubble && optBubble.parentNode) optBubble.remove();
                        queuedPrompts = Array.isArray(data.queue) ? data.queue : queuedPrompts;
                        renderQueuedPromptsTray();
                    }
                } catch (err) {
                    if (optBubble && optBubble.parentNode && optBubble.dataset.optimistic === '1') {
                        optBubble.remove();
                    }
                    setSessionRunning(false);
                    appendSystemError('Error: ' + err.message);
                }
            } else {
                try {
                    const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/message`, {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({
                            content,
                            attachments: attachmentsToSend,
                            slash_command: slashCommand,
                            slash_args: slashArgs,
                            mode: activeSendMode,
                            mock: mockModeEnabled
                        })
                    });
                    if (!res.ok) {
                        const errText = await res.text().catch(() => '');
                        throw new Error(errText || `HTTP ${res.status}`);
                    }
                    const data = await res.json();
                    if (data.status === 'queued') {
                        queuedPrompts = Array.isArray(data.queue) ? data.queue : queuedPrompts;
                        renderQueuedPromptsTray();
                    } else if (data.status === 'running' && data.turn_id) {
                        setSessionRunning(true);
                        activeTurnId = Number(data.turn_id);
                        if (!slashCommand && (content || attachmentsToSend.length > 0)) {
                            renderOrUpgradeUserMessage(data.turn_id, content, attachmentsToSend);
                        }
                    }
                } catch (err) {
                    appendSystemError('Error: ' + err.message);
                }
            }
            return;
        }

        // Brand new conversation -> POST /api/sessions
        setSessionRunning(true);
        if (welcomeCanvas) welcomeCanvas.style.display = 'none';
        resetInspectorState();
        activeReasoningDrawer = null;
        activeMessageBox = null;
        activeTurnId = 1;
        queuedPrompts = [];
        renderQueuedPromptsTray();

        const attLabel = attachmentsToSend.length === 1
            ? `Attached ${attachmentsToSend[0].name}`
            : (attachmentsToSend.length > 1 ? `Attached ${attachmentsToSend.length} files` : 'New Session');
        const taskDisplayTitle = slashCommand
            ? (content ? `/${slashCommand} ${content}` : `/${slashCommand}`)
            : (content || attLabel);
        const optBubble = (!slashCommand && (content || attachmentsToSend.length > 0))
            ? appendUserMessage(content, 1, false, attachmentsToSend)
            : null;
        if (clearInput) {
            taskInput.value = '';
            autoResizeTextarea();
            clearComposerSlashCommand();
            clearStagedAttachments();
        }

        try {
            const res = await fetch('/api/sessions', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    task: content || (slashCommand ? taskDisplayTitle : ''),
                    attachments: attachmentsToSend,
                    slash_command: slashCommand,
                    slash_args: slashArgs,
                    mock: mockModeEnabled,
                    project_id: activeComposerScope === 'conversation' ? CONVERSATIONS_PROJECT_ID : activeProjectId,
                    server_id: (currentConfig && currentConfig.active_server_id) || 'local',
                    orchestrator_mode: Boolean(currentConfig && currentConfig.orchestrator && currentConfig.orchestrator.enabled),
                    model: (currentConfig && currentConfig.model) || '',
                    provider_id: (currentConfig && currentConfig.provider) || '',
                    reasoning_effort: (currentConfig && currentConfig.reasoning_effort) || ''
                })
            });

            // Only a NON-OK response is a failure: a 200 must fall through to the session.
            // (Getting this backwards makes every first prompt fail with
            // "Could not start a session (HTTP 200)".)
            if (!res.ok) {
                // Name the fix, not just the failure: the most common cause is an
                // unconfigured provider, and the user needs to know where to go.
                let detail = '';
                try { detail = (await res.text()) || ''; } catch (_) {}
                let hint = 'Check Settings → AI Providers for a valid API key and model.';
                const low = detail.toLowerCase();
                if (low.includes('provider') || low.includes('api key') || low.includes('no model')) {
                    hint = 'No API key or model configured. Settings → AI Providers → add a provider, then try again.';
                } else if (low.includes('project')) {
                    hint = 'The selected project is missing or unusable. Pick another project in the sidebar.';
                } else if (res.status === 429) {
                    hint = 'The model provider is rate-limiting this key. Wait a moment, or use another provider.';
                } else if (res.status === 401 || res.status === 403) {
                    hint = 'The API key was rejected. Settings → AI Providers → re-enter it.';
                }
                throw new Error(`Could not start a session (HTTP ${res.status}). ${hint}`);
            }
            const { session_id } = await res.json();
            if (!session_id) {
                throw new Error('The server did not return a session id. Try again.');
            }
            currentSessionId = session_id;
            updateBreadcrumbSessionUI(taskDisplayTitle);

            connectEventStream(session_id);
            await fetchSessions();
            await fetchProjects();
        } catch (err) {
            if (optBubble && optBubble.parentNode) optBubble.remove();
            appendSystemError('Error: ' + err.message);
            setSessionRunning(false);
        }
    } finally {
        isSubmittingPrompt = false;
        updateSubmitButtonState();
    }
}

// ==========================================================================
// Command Palette (Ctrl+K)
// ==========================================================================
let paletteItems = [];
let paletteSelectedIndex = 0;

function openCommandPalette() {
    closeAllPopovers();
    if (!commandPaletteModal) return;
    commandPaletteModal.classList.remove('hidden');
    paletteSearchInput.value = '';
    paletteSelectedIndex = 0;
    buildAndRenderPaletteResults('');
    paletteSearchInput.focus();
}

function closeCommandPalette() {
    if (commandPaletteModal) commandPaletteModal.classList.add('hidden');
}

function buildPaletteCommands() {
    const commands = [
        {
            category: 'Actions',
            title: 'New Conversation',
            sub: 'Start a clean agentic session',
            icon: '＋',
            action: () => startNewConversation()
        },
        {
            category: 'Actions',
            title: 'Toggle Right Inspector (Swarm / Browser / Files)',
            sub: 'Open or collapse the Inspector pane',
            icon: '◫',
            action: () => {
                if (inspectorPane.classList.contains('collapsed')) {
                    userClosedInspector = false;
                    openInspector();
                } else {
                    userClosedInspector = true;
                    closeInspector();
                }
            }
        },
        {
            category: 'Projects',
            title: 'Create New Project Workspace',
            sub: 'Add an isolated sandboxed workspace directory',
            icon: '📁',
            action: () => openNewProjectModal()
        },
        ...allProjects.map(p => ({
            category: 'Projects',
            title: `Switch Project: ${p.name}`,
            sub: p.path,
            icon: p.icon || '📁',
            badge: p.id === activeProjectId ? 'ACTIVE' : undefined,
            action: () => switchActiveProject(p.id)
        })),
        ...allSkills.filter(s => s.enabled !== false).map(s => ({
            category: 'Skills',
            title: `/${s.name}`,
            sub: s.description || 'Insert skill slash command',
            icon: getCommandSvgIcon('skill', 14),
            badge: 'SKILL',
            action: () => {
                setComposerSlashCommand(s.name, { label: s.description || s.name, iconKey: 'skill' });
                autoResizeTextarea();
                taskInput.focus();
            }
        })),
        {
            category: 'Configuration',
            title: 'Customizations & Skills Manager',
            sub: 'Manage .harness/skills/*.md and tool permissions',
            icon: '✨',
            action: () => openCustomizationsModal('skills')
        },
        {
            category: 'Configuration',
            title: 'AI Providers & Models',
            sub: 'Configure NVIDIA NIM, OpenAI, Anthropic, DeepSeek, Ollama',
            icon: '🌐',
            action: () => openSettingsModal('providers')
        },
        {
            category: 'Configuration',
            title: 'MCP Servers & Google AI Search',
            sub: 'Manage built-in Google AI Search MCP and stdio / SSE servers',
            icon: '🔌',
            action: () => openSettingsModal('mcp')
        },
        {
            category: 'Configuration',
            title: 'Search Engines (TinyFish, Tavily, DuckDuckGo)',
            sub: 'Live web scraping and search provider configuration',
            icon: '🔍',
            action: () => openSettingsModal('search')
        },
        {
            category: 'Configuration',
            title: 'Tools & Security Permissions',
            sub: 'Auto-approve, Ask before executing, or Read-only policy',
            icon: '🛡',
            action: () => openSettingsModal('tools')
        },
        {
            category: 'Configuration',
            title: 'Registered Plugin Tools',
            sub: 'Inspect active Rust tools: bash, browser_ui, invoke_subagent, etc.',
            icon: '🧩',
            action: () => openToolsModal()
        },
        {
            category: 'System',
            title: 'Settings: Memory & Runtime',
            sub: 'Context budgets, auto-compaction, retries, workspace root',
            icon: '⚙',
            action: () => openSettingsModal('memory')
        },
        {
            category: 'System',
            title: 'Skills & Automations',
            sub: 'Background cron jobs and .harness/skills/*.md commands',
            icon: '⏱',
            action: () => openSettingsModal('skills')
        },
        {
            category: 'Actions',
            title: 'Clear Chat Canvas',
            sub: 'Wipe current message stream',
            icon: '🗑',
            action: () => startNewConversation()
        },
        {
            category: 'Actions',
            title: 'Export Session JSON',
            sub: 'Download conversation history and events as JSON',
            icon: '📥',
            action: () => exportSessionJson()
        }
    ];

    allSessions.slice(0, 12).forEach(s => {
        commands.push({
            category: 'Sessions',
            title: s.task,
            sub: `Created ${formatTimeAgo(s.created_at)} · ${s.total_steps} steps`,
            icon: '💬',
            action: () => loadHistoricalSession(s.id, s.task)
        });
    });

    return commands;
}

function buildAndRenderPaletteResults(query) {
    const q = query.toLowerCase().trim();
    const all = buildPaletteCommands();

    paletteItems = all.filter(item => {
        if (!q) return true;
        return item.title.toLowerCase().includes(q) ||
            (item.sub && item.sub.toLowerCase().includes(q)) ||
            (item.category && item.category.toLowerCase().includes(q));
    });

    if (paletteSelectedIndex >= paletteItems.length) {
        paletteSelectedIndex = Math.max(0, paletteItems.length - 1);
    }

    renderPaletteDOM();
}

function renderPaletteDOM() {
    if (!paletteResultsList) return;
    paletteResultsList.innerHTML = '';

    if (paletteItems.length === 0) {
        paletteResultsList.innerHTML = '<div style="padding: 24px; text-align: center; color: var(--text-dim); font-size: 0.85rem;">No matching commands or sessions found</div>';
        return;
    }

    let lastCategory = '';
    paletteItems.forEach((item, index) => {
        if (item.category && item.category !== lastCategory) {
            lastCategory = item.category;
            const catHeader = document.createElement('div');
            catHeader.className = 'palette-category-header';
            catHeader.textContent = lastCategory;
            paletteResultsList.appendChild(catHeader);
        }

        const row = document.createElement('div');
        const isSelected = index === paletteSelectedIndex;
        row.className = `palette-item ${isSelected ? 'selected' : ''}`;
        row.innerHTML = `
            <span class="palette-item-icon">${item.icon}</span>
            <div class="palette-item-main">
                <span class="palette-item-title">${escapeHtml(item.title)}</span>
                ${item.sub ? `<span class="palette-item-sub">${escapeHtml(item.sub)}</span>` : ''}
            </div>
            ${item.badge ? `<span class="palette-item-badge">${escapeHtml(item.badge)}</span>` : ''}
        `;

        row.addEventListener('click', () => {
            closeCommandPalette();
            item.action();
        });
        row.addEventListener('mouseenter', () => {
            paletteSelectedIndex = index;
            updatePaletteSelection();
        });
        paletteResultsList.appendChild(row);
    });

    scrollSelectedPaletteIntoView();
}

function updatePaletteSelection() {
    if (!paletteResultsList) return;
    paletteResultsList.querySelectorAll('.palette-item').forEach((el, idx) => {
        el.classList.toggle('selected', idx === paletteSelectedIndex);
    });
}

function scrollSelectedPaletteIntoView() {
    if (!paletteResultsList) return;
    const selected = paletteResultsList.querySelector('.palette-item.selected');
    if (selected) selected.scrollIntoView({ block: 'nearest' });
}

function handlePaletteKeydown(e) {
    if (paletteItems.length === 0) return;

    if (e.key === 'ArrowDown') {
        e.preventDefault();
        paletteSelectedIndex = (paletteSelectedIndex + 1) % paletteItems.length;
        updatePaletteSelection();
        scrollSelectedPaletteIntoView();
    } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        paletteSelectedIndex = (paletteSelectedIndex - 1 + paletteItems.length) % paletteItems.length;
        updatePaletteSelection();
        scrollSelectedPaletteIntoView();
    } else if (e.key === 'Enter') {
        e.preventDefault();
        const selected = paletteItems[paletteSelectedIndex];
        if (selected) {
            closeCommandPalette();
            selected.action();
        }
    } else if (e.key === 'Escape') {
        e.preventDefault();
        closeCommandPalette();
    }
}

// ==========================================================================
// Uniform Inline SVG Icon Registry & Display Verbosity Helpers
// ==========================================================================
function getCommandSvgIcon(key, size = 14) {
    const k = String(key || 'default').replace(/^[/@]+/, '').toLowerCase();
    const base = `width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"`;

    const paths = {
        swarm: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/>',
        invoke_subagent: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/>',
        swarm_execute: '<circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/>',
        swarm_board: '<path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/>',
        plan: '<line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/>',
        compact: '<polyline points="4 14 10 14 10 20"/><polyline points="20 10 14 10 14 4"/><line x1="14" y1="10" x2="21" y2="3"/><line x1="3" y1="21" x2="10" y2="14"/>',
        debug: '<path d="M8 2l1.88 1.88"/><path d="M14.12 3.88L16 2"/><path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1"/><path d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6"/><path d="M12 20v-9"/><path d="M6.53 9C4.6 8.8 3 7.1 3 5"/><path d="M6 13H2"/><path d="M3 21c0-2.1 1.7-3.9 3.8-4"/><path d="M20.97 5c0 2.1-1.6 3.8-3.5 4"/><path d="M22 13h-4"/><path d="M17.2 17c2.1.1 3.8 1.9 3.8 4"/>',
        diagnose: '<polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>',
        sys_info: '<polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>',
        search: '<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>',
        web_search: '<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>',
        grep_search: '<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>',
        review: '<path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/>',
        test: '<path d="M9 3h6"/><path d="M10 9h4"/><path d="M10 3v6l-4 8a2 2 0 0 0 1.8 3h8.4a2 2 0 0 0 1.8-3l-4-8V3"/>',
        browser: '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>',
        browser_ui: '<circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>',
        web_fetch: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
        summarize: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/>',
        read_file: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/>',
        write_file: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="12" y1="18" x2="12" y2="12"/><line x1="9" y1="15" x2="15" y2="15"/>',
        edit_file: '<path d="M12 20h9"/><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"/>',
        bash: '<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>',
        list_dir: '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>',
        create_artifact: '<path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z"/>',
        workspace: '<path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/>',
        clear: '<polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
        skill: '<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>',
        default: '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>'
    };

    const inner = paths[k] || paths.default;
    return `<svg ${base}>${inner}</svg>`;
}

function getToolSvgIcon(toolName) {
    return getCommandSvgIcon(toolName || 'default', 13);
}

function normalizeDisplayVerbosity(val) {
    const v = String(val || '').trim().toLowerCase();
    if (v === 'balanced' || v === 'verbose' || v === 'compact') return v;
    return 'compact';
}

function setDisplayVerbosity(level, persist = false) {
    const normalized = normalizeDisplayVerbosity(level);
    activeDisplayVerbosity = normalized;
    if (document.body) document.body.dataset.verbosity = normalized;

    if (settingsDisplayVerbosity) {
        settingsDisplayVerbosity.querySelectorAll('[data-verbosity]').forEach(btn => {
            btn.classList.toggle('active', btn.getAttribute('data-verbosity') === normalized);
        });
    }

    applyDisplayVerbosityToStream(true);

    if (persist && currentConfig) {
        currentConfig.display_verbosity = normalized;
        persistDisplayVerbosityConfig();
    }
}

async function persistDisplayVerbosityConfig() {
    if (!currentConfig) return;
    try {
        await saveCurrentConfig();
    } catch (_) {}
}

function applyDisplayVerbosityToStream(resetUserToggles = false) {
    if (!streamContainer) return;

    streamContainer.querySelectorAll('.reasoning-accordion').forEach(drawer => {
        if (resetUserToggles) delete drawer.dataset.userToggled;
        if (drawer.dataset.userToggled === '1') return;
        drawer.open = (activeDisplayVerbosity === 'verbose');
    });

    streamContainer.querySelectorAll('.tool-card').forEach(card => {
        if (resetUserToggles) delete card.dataset.userToggled;
        if (card.dataset.userToggled === '1') return;
        const isFailure = card.dataset.success === 'false';
        const shouldExpand = activeDisplayVerbosity === 'verbose'
            || (activeDisplayVerbosity === 'balanced' && isFailure);
        card.classList.toggle('expanded', shouldExpand);
    });
}

// ==========================================================================
// Prompt Autocomplete (@ Mentions & Dynamic / Skills + Slash Commands)
// ==========================================================================
const MENTION_OPTIONS = [
    { name: '@bash', desc: 'Shell command execution & terminal', iconKey: 'bash' },
    { name: '@read_file', desc: 'Read file contents in workspace', iconKey: 'read_file' },
    { name: '@write_file', desc: 'Create or overwrite files', iconKey: 'write_file' },
    { name: '@edit_file', desc: 'Surgical string replacement', iconKey: 'edit_file' },
    { name: '@browser_ui', desc: 'Headless browser, screenshot & DOM audit', iconKey: 'browser_ui' },
    { name: '@invoke_subagent', desc: 'Delegate parallel subagent task', iconKey: 'invoke_subagent' },
    { name: '@swarm_board', desc: 'Post or read shared swarm board', iconKey: 'swarm_board' },
    { name: '@create_artifact', desc: 'Save markdown/plan/report artifact', iconKey: 'create_artifact' },
    { name: '@web_search', desc: 'Real-time live web search', iconKey: 'web_search' },
    { name: '@web_fetch', desc: 'Scrape URL and parse to markdown', iconKey: 'web_fetch' },
    { name: '@list_dir', desc: 'Inspect directory layout', iconKey: 'list_dir' },
    { name: '@grep_search', desc: 'Recursive regex codebase search', iconKey: 'grep_search' },
    { name: '@sys_info', desc: 'Host CPU, RAM, & telemetry', iconKey: 'sys_info' },
    { name: '@workspace', desc: 'Current project workspace root', iconKey: 'workspace' }
];

const BUILTIN_SLASH_OPTIONS = [
    { name: '/swarm', label: 'Multi-Agent Swarm', desc: 'Delegate multi-agent swarm workflow', iconKey: 'swarm' },
    { name: '/plan', label: 'Architecture Plan', desc: 'Formulate step-by-step architecture plan', iconKey: 'plan' },
    { name: '/compact', label: 'Compact Context', desc: 'Summarize and compact active context window', iconKey: 'compact' },
    { name: '/debug', label: 'Debug & Diagnose', desc: 'Analyze failures, logs & root cause', iconKey: 'debug' },
    { name: '/search', label: 'Live Web Search', desc: 'Perform live web documentation search', iconKey: 'search' },
    { name: '/review', label: 'Code Review', desc: 'Audit diff, edge cases & code quality', iconKey: 'review' },
    { name: '/diagnose', label: 'System Diagnose', desc: 'Inspect system logs, errors & runtime health', iconKey: 'diagnose' },
    { name: '/test', label: 'Run Tests', desc: 'Run test suite and verify integrity', iconKey: 'test' },
    { name: '/browser', label: 'Browser UI Audit', desc: 'Audit UI with browser_ui screenshot & DOM check', iconKey: 'browser' },
    { name: '/summarize', label: 'Summarize Changes', desc: 'Summarize active workspace changes', iconKey: 'summarize' },
    { name: '/condense', label: 'Condense into Skill', desc: 'Condense this session workflow into a reusable skill', iconKey: 'skill' },
    { name: '/clear', label: 'Clear Session', desc: 'Reset conversation history and canvas', iconKey: 'clear' }
];

function getSlashOptions() {
    const merged = [...BUILTIN_SLASH_OPTIONS];
    const seen = new Set(merged.map(x => x.name.toLowerCase()));
    allSkills.filter(s => s.enabled !== false).forEach(skill => {
        const cmd = `/${skill.name}`;
        if (!seen.has(cmd.toLowerCase())) {
            seen.add(cmd.toLowerCase());
            merged.unshift({
                name: cmd,
                label: skill.name,
                desc: skill.description || 'Workspace skill',
                iconKey: 'skill'
            });
        }
    });
    return merged;
}

let activeAutocompleteType = null;
let activeAutocompleteQuery = '';
let activeAutocompleteIndex = 0;
let filteredAutocompleteItems = [];

function checkPromptAutocomplete() {
    if (!taskInput || !promptAutocompleteMenu) return;

    const cursor = taskInput.selectionStart;
    const textBefore = taskInput.value.slice(0, cursor);

    const atMatch = textBefore.match(/@([a-zA-Z0-9_-]*)$/);
    if (atMatch) {
        activeAutocompleteType = '@';
        activeAutocompleteQuery = atMatch[1].toLowerCase();
        filteredAutocompleteItems = MENTION_OPTIONS.filter(opt =>
            opt.name.slice(1).toLowerCase().includes(activeAutocompleteQuery) ||
            opt.desc.toLowerCase().includes(activeAutocompleteQuery)
        );
        showAutocompleteMenu('Mention Tool or Resource');
        return;
    }

    const slashMatch = textBefore.match(/(?:^|\s)\/([a-zA-Z0-9_-]*)$/);
    if (slashMatch) {
        activeAutocompleteType = '/';
        activeAutocompleteQuery = slashMatch[1].toLowerCase();
        filteredAutocompleteItems = getSlashOptions().filter(opt =>
            opt.name.slice(1).toLowerCase().includes(activeAutocompleteQuery) ||
            opt.desc.toLowerCase().includes(activeAutocompleteQuery)
        );
        showAutocompleteMenu('Skills & Slash Commands');
        return;
    }

    hideAutocompleteMenu();
}

function showAutocompleteMenu(headerTitle) {
    if (filteredAutocompleteItems.length === 0) {
        hideAutocompleteMenu();
        return;
    }

    if (autocompleteHeader) autocompleteHeader.textContent = headerTitle;
    activeAutocompleteIndex = 0;
    renderAutocompleteDOM();

    const promptDock = document.querySelector('.prompt-dock-container');
    if (promptDock) {
        const rect = promptDock.getBoundingClientRect();
        promptAutocompleteMenu.style.left = `${rect.left + 20}px`;
        promptAutocompleteMenu.style.bottom = `${window.innerHeight - rect.top + 10}px`;
    }
    promptAutocompleteMenu.classList.remove('hidden');
}

function hideAutocompleteMenu() {
    if (promptAutocompleteMenu) promptAutocompleteMenu.classList.add('hidden');
    activeAutocompleteType = null;
    filteredAutocompleteItems = [];
}

function renderAutocompleteDOM() {
    if (!autocompleteList) return;
    autocompleteList.innerHTML = '';

    filteredAutocompleteItems.forEach((item, index) => {
        const row = document.createElement('div');
        row.className = `autocomplete-item ${index === activeAutocompleteIndex ? 'active' : ''}`;
        row.innerHTML = `
            <span class="autocomplete-item-icon">${getCommandSvgIcon(item.iconKey || 'skill', 14)}</span>
            <span class="autocomplete-item-name">${escapeHtml(item.name)}</span>
            <span class="autocomplete-item-desc">${escapeHtml(item.desc)}</span>
        `;
        row.addEventListener('click', () => selectAutocompleteItem(item));
        row.addEventListener('mouseenter', () => {
            activeAutocompleteIndex = index;
            updateAutocompleteSelection();
        });
        autocompleteList.appendChild(row);
    });
}

function updateAutocompleteSelection() {
    if (!autocompleteList) return;
    autocompleteList.querySelectorAll('.autocomplete-item').forEach((row, idx) => {
        row.classList.toggle('active', idx === activeAutocompleteIndex);
    });
}

function selectAutocompleteItem(item) {
    if (!taskInput || !activeAutocompleteType) return;
    const cursor = taskInput.selectionStart;
    const text = taskInput.value;
    const textBefore = text.slice(0, cursor);
    const textAfter = text.slice(cursor);

    const replaceStart = activeAutocompleteType === '@'
        ? textBefore.lastIndexOf('@')
        : textBefore.lastIndexOf('/');

    if (activeAutocompleteType === '/') {
        if (replaceStart !== -1) {
            const beforeTrimmed = text.slice(0, replaceStart).replace(/\s+$/, '');
            const afterTrimmed = textAfter.replace(/^\s+/, '');
            taskInput.value = beforeTrimmed && afterTrimmed
                ? `${beforeTrimmed} ${afterTrimmed}`
                : `${beforeTrimmed}${afterTrimmed}`;
            const nextPos = beforeTrimmed.length;
            taskInput.setSelectionRange(nextPos, nextPos);
        }
        if (item.name === '/clear') {
            hideAutocompleteMenu();
            startNewConversation();
            return;
        }
        setComposerSlashCommand(item.name.slice(1), item);
        autoResizeTextarea();
        taskInput.focus();
        hideAutocompleteMenu();
        return;
    }

    if (replaceStart !== -1) {
        const newText = text.slice(0, replaceStart) + item.name + ' ' + textAfter;
        taskInput.value = newText;
        const newCursor = replaceStart + item.name.length + 1;
        taskInput.setSelectionRange(newCursor, newCursor);
        autoResizeTextarea();
        taskInput.focus();
    }
    hideAutocompleteMenu();
}

function handleTaskInputKeydown(e) {
    if (promptAutocompleteMenu && !promptAutocompleteMenu.classList.contains('hidden') && filteredAutocompleteItems.length > 0) {
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            activeAutocompleteIndex = (activeAutocompleteIndex + 1) % filteredAutocompleteItems.length;
            updateAutocompleteSelection();
            return;
        }
        if (e.key === 'ArrowUp') {
            e.preventDefault();
            activeAutocompleteIndex = (activeAutocompleteIndex - 1 + filteredAutocompleteItems.length) % filteredAutocompleteItems.length;
            updateAutocompleteSelection();
            return;
        }
        if (e.key === 'Enter' || e.key === 'Tab') {
            e.preventDefault();
            selectAutocompleteItem(filteredAutocompleteItems[activeAutocompleteIndex]);
            return;
        }
        if (e.key === 'Escape') {
            e.preventDefault();
            hideAutocompleteMenu();
            return;
        }
    }

    if (e.key === 'Backspace' && selectedSlashCommand && taskInput.selectionStart === 0 && taskInput.selectionEnd === 0 && !taskInput.value) {
        e.preventDefault();
        clearComposerSlashCommand();
        return;
    }

    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handlePromptSubmit();
    }
}

function handleGlobalKeydown(e) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        if (commandPaletteModal.classList.contains('hidden')) openCommandPalette();
        else closeCommandPalette();
        return;
    }

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        startNewConversation();
        return;
    }

    if (e.key === 'Escape') {
        closeAllPopovers();
        if (commandPaletteModal && !commandPaletteModal.classList.contains('hidden')) closeCommandPalette();
        if (projectModal && !projectModal.classList.contains('hidden')) closeNewProjectModal();
        if (configModal && !configModal.classList.contains('hidden')) configModal.classList.add('hidden');
        if (toolsModal && !toolsModal.classList.contains('hidden')) toolsModal.classList.add('hidden');
        if (imageLightboxModal && !imageLightboxModal.classList.contains('hidden')) imageLightboxModal.classList.add('hidden');
        if (promptAutocompleteMenu && !promptAutocompleteMenu.classList.contains('hidden')) hideAutocompleteMenu();
    }
}

// ==========================================================================
// Stream Canvas, Queued Prompts, Per-Message Actions & SSE Event Dispatch
// ==========================================================================
async function copyTextToClipboard(text) {
    const val = text == null ? '' : String(text);
    if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
        try {
            await navigator.clipboard.writeText(val);
            return true;
        } catch (_) {}
    }
    try {
        const ta = document.createElement('textarea');
        ta.value = val;
        ta.style.position = 'fixed';
        ta.style.opacity = '0';
        document.body.appendChild(ta);
        ta.select();
        const ok = document.execCommand('copy');
        ta.remove();
        return ok;
    } catch (_) {
        return false;
    }
}

function getCopySvgIcon() {
    return `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-1px"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
}

function flashCopiedState(btn, restoredHtml) {
    if (!btn) return;
    const orig = restoredHtml !== undefined ? restoredHtml : btn.innerHTML;
    btn.textContent = '✓ Copied';
    btn.classList.add('copied');
    setTimeout(() => {
        btn.innerHTML = orig;
        btn.classList.remove('copied');
    }, 1400);
}

function appendUserMessage(text, turnId = null, isOptimistic = false, attachments = []) {
    const bubble = document.createElement('div');
    bubble.className = 'message-bubble user-msg user-msg-bubble';
    bubble.dataset.rawText = text || '';
    if (turnId) bubble.dataset.turnId = String(turnId);
    if (isOptimistic) bubble.dataset.optimistic = '1';

    const attHtml = renderAttachmentListHtml(attachments);
    const textHtml = (text || '').trim()
        ? `<div class="user-msg-content">${escapeHtml(text)}</div>`
        : '';

    bubble.innerHTML = `
        ${attHtml}
        ${textHtml}
        <div class="msg-action-bar user-msg-actions">
            <button type="button" class="msg-action-btn" data-action="copy" title="Copy message">${getCopySvgIcon()} Copy</button>
            <button type="button" class="msg-action-btn" data-action="edit" title="Edit &amp; resend from here">✎ Edit</button>
            <button type="button" class="msg-action-btn" data-action="undo" title="Undo conversation up to this message">↩ Undo</button>
            <button type="button" class="msg-action-btn" data-action="fork" title="Fork conversation from here">⑂ Fork</button>
        </div>
    `;
    bubble.querySelectorAll('.user-att-img-card').forEach(btn => {
        btn.addEventListener('click', () => {
            const src = btn.getAttribute('data-lightbox-src');
            const title = btn.getAttribute('data-lightbox-title') || 'Attached Image';
            if (src) openImageLightbox(src, title);
        });
    });
    wireUserBubbleActions(bubble);
    streamContainer.appendChild(bubble);
    scrollToBottom();
    return bubble;
}

function upgradeUserBubbleTurnId(bubble, turnId) {
    if (!bubble) return;
    if (turnId) bubble.dataset.turnId = String(turnId);
    delete bubble.dataset.optimistic;
}

function renderOrUpgradeUserMessage(turnId, content, attachments = []) {
    if (turnId) {
        const existing = streamContainer.querySelector(`.user-msg-bubble[data-turn-id="${turnId}"]`);
        if (existing) {
            delete existing.dataset.optimistic;
            return existing;
        }
    }
    const opt = streamContainer.querySelector('.user-msg-bubble[data-optimistic="1"]');
    if (opt && (opt.dataset.rawText || '').trim() === (content || '').trim()) {
        upgradeUserBubbleTurnId(opt, turnId);
        return opt;
    }
    return appendUserMessage(content, turnId, false, attachments);
}

function wireUserBubbleActions(bubble) {
    const copyBtn = bubble.querySelector('[data-action="copy"]');
    const editBtn = bubble.querySelector('[data-action="edit"]');
    const undoBtn = bubble.querySelector('[data-action="undo"]');
    const forkBtn = bubble.querySelector('[data-action="fork"]');

    if (copyBtn) {
        copyBtn.addEventListener('click', async () => {
            const raw = bubble.dataset.rawText || bubble.querySelector('.user-msg-content')?.textContent || '';
            if (await copyTextToClipboard(raw)) flashCopiedState(copyBtn, `${getCopySvgIcon()} Copy`);
        });
    }
    if (editBtn) {
        editBtn.addEventListener('click', () => {
            const tid = Number(bubble.dataset.turnId);
            if (!currentSessionId || !tid) return;
            undoToTurn(tid, 'edit');
        });
    }
    if (undoBtn) {
        undoBtn.addEventListener('click', () => {
            const tid = Number(bubble.dataset.turnId);
            if (!currentSessionId || !tid) return;
            undoToTurn(tid, 'undo');
        });
    }
    if (forkBtn) {
        forkBtn.addEventListener('click', () => {
            const tid = Number(bubble.dataset.turnId);
            if (!currentSessionId || !tid) return;
            forkCurrentSession(tid);
        });
    }
}

function wireAssistantBubbleActions(bubble) {
    const copyBtn = bubble.querySelector('[data-action="copy"]');
    const retryBtn = bubble.querySelector('[data-action="retry"]');
    const forkBtn = bubble.querySelector('[data-action="fork"]');
    const undoBtn = bubble.querySelector('[data-action="undo"]');

    if (copyBtn) {
        copyBtn.addEventListener('click', async () => {
            const contentEl = bubble.querySelector('.assistant-msg-content');
            const raw = contentEl ? (contentEl.getAttribute('data-raw') || contentEl.textContent || '') : '';
            if (await copyTextToClipboard(raw)) flashCopiedState(copyBtn, `${getCopySvgIcon()} Copy`);
        });
    }
    if (retryBtn) {
        retryBtn.addEventListener('click', () => {
            const tid = Number(bubble.dataset.turnId);
            if (!currentSessionId || !tid) return;
            undoToTurn(tid, 'retry');
        });
    }
    if (forkBtn) {
        forkBtn.addEventListener('click', () => {
            const tid = Number(bubble.dataset.turnId);
            if (!currentSessionId || !tid) return;
            forkCurrentSession(tid);
        });
    }
    if (undoBtn) {
        undoBtn.addEventListener('click', () => {
            const tid = Number(bubble.dataset.turnId);
            if (!currentSessionId || !tid) return;
            undoToTurn(tid, 'undo');
        });
    }
}

function appendSteeringPill(content) {
    const pill = document.createElement('div');
    pill.className = 'steering-applied-pill';
    pill.innerHTML = `<span>⚡</span><span><strong>Steer:</strong> ${escapeHtml(content)}</span>`;
    streamContainer.appendChild(pill);
}

function renderQueuedPromptsTray() {
    if (!queuedPromptsTray) return;
    const list = Array.isArray(queuedPrompts) ? queuedPrompts : [];
    if (list.length === 0) {
        queuedPromptsTray.classList.add('hidden');
        queuedPromptsTray.innerHTML = '';
        return;
    }

    queuedPromptsTray.classList.remove('hidden');
    queuedPromptsTray.innerHTML = `
        <div class="queued-tray-header">
            <span>Queued Prompts (${list.length})</span>
            <span>Runs next in order · or promote to Steer</span>
        </div>
        <div class="queued-tray-list"></div>
    `;
    const listEl = queuedPromptsTray.querySelector('.queued-tray-list');
    list.forEach((item, idx) => {
        const qid = item.id || item.queue_id || '';
        const text = item.content || '';
        const row = document.createElement('div');
        row.className = 'queued-prompt-row';
        row.dataset.queueId = qid;
        row.innerHTML = `
            <span class="queued-pos-badge">#${idx + 1}</span>
            <span class="queued-prompt-preview" title="${escapeHtml(text)}">${escapeHtml(text)}</span>
            <div class="queued-prompt-actions">
                <button type="button" class="queue-item-btn steer" title="Promote immediately to mid-run Steer">⚡ Steer</button>
                <button type="button" class="queue-item-btn remove" title="Remove from queue">✕</button>
            </div>
        `;
        const steerBtn = row.querySelector('.queue-item-btn.steer');
        const removeBtn = row.querySelector('.queue-item-btn.remove');
        if (steerBtn) steerBtn.addEventListener('click', () => promoteQueuedPromptToSteer(qid));
        if (removeBtn) removeBtn.addEventListener('click', () => removeQueuedPrompt(qid));
        listEl.appendChild(row);
    });
}

async function fetchSessionQueue(sessionId) {
    if (!sessionId) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(sessionId)}/queue`);
        if (!res.ok) return;
        const data = await res.json();
        queuedPrompts = Array.isArray(data.queue) ? data.queue : [];
        renderQueuedPromptsTray();
    } catch (_) {}
}

async function promoteQueuedPromptToSteer(queueId) {
    if (!currentSessionId || !queueId) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/queue/${encodeURIComponent(queueId)}/steer`, {
            method: 'POST'
        });
        if (!res.ok) return;
        const data = await res.json();
        queuedPrompts = Array.isArray(data.queue) ? data.queue : [];
        renderQueuedPromptsTray();
    } catch (e) {
        console.error('Failed to promote queued prompt to steer', e);
    }
}

async function removeQueuedPrompt(queueId) {
    if (!currentSessionId || !queueId) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/queue/${encodeURIComponent(queueId)}`, {
            method: 'DELETE'
        });
        if (!res.ok) return;
        const data = await res.json();
        queuedPrompts = Array.isArray(data.queue) ? data.queue : [];
        renderQueuedPromptsTray();
    } catch (e) {
        console.error('Failed to remove queued prompt', e);
    }
}

async function undoToTurn(turnId, mode = 'undo') {
    if (!currentSessionId || !turnId) return;
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/undo`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ turn_id: turnId })
        });
        if (!res.ok) {
            const errText = await res.text().catch(() => '');
            throw new Error(errText || `HTTP ${res.status}`);
        }
        const data = await res.json();
        queuedPrompts = [];
        renderQueuedPromptsTray();
        setSessionRunning(false);
        replayHistoryEvents(Array.isArray(data.history) ? data.history : []);
        await fetchSessions();

        if (mode === 'edit') {
            taskInput.value = data.restored_prompt || '';
            autoResizeTextarea();
            taskInput.focus();
        } else if (mode === 'retry') {
            await submitPromptText(data.restored_prompt || '', false);
        }
    } catch (err) {
        appendSystemError('Undo failed: ' + err.message);
    }
}

async function forkCurrentSession(turnId = null) {
    if (!currentSessionId) {
        showToast('No active conversation to fork.', 'warning');
        return;
    }
    try {
        const payload = turnId ? { turn_id: turnId } : {};
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/fork`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        if (!res.ok) {
            const errText = await res.text().catch(() => '');
            throw new Error(errText || `HTTP ${res.status}`);
        }
        const data = await res.json();
        await fetchSessions();
        await fetchProjects();
        await loadHistoricalSession(data.session_id, data.title);
    } catch (err) {
        appendSystemError('Fork failed: ' + err.message);
    }
}

function finalizeOrphanedStreamStates() {
    if (!streamContainer) return;
    streamContainer.querySelectorAll('.tool-status-pill.running, .tool-status-pill.retrying').forEach(pill => {
        pill.className = 'tool-status-pill interrupted';
        pill.textContent = 'Stopped';
        const card = pill.closest('.tool-card');
        if (card && !card.dataset.success) {
            card.dataset.success = 'false';
        }
    });
    streamContainer.querySelectorAll('.reasoning-accordion .pulse-dot').forEach(dot => {
        dot.classList.add('idle');
    });
}

function replayHistoryEvents(events) {
    streamContainer.innerHTML = '';
    // Any outstanding approval belonged to the stream we just discarded.
    clearAllPendingPermissions();
    activeReasoningDrawer = null;
    activeMessageBox = null;
    activeTurnId = null;
    resetInspectorState();

    if (!events || events.length === 0) {
        if (welcomeCanvas) {
            welcomeCanvas.style.display = 'flex';
            streamContainer.appendChild(welcomeCanvas);
        }
        return;
    }

    if (welcomeCanvas) welcomeCanvas.style.display = 'none';
    events.forEach(ev => handleHarnessEvent(ev, true));
    scrollToBottom(true);
}

function startInlineRenameSession() {
    if (!currentSessionId || !currentTaskBreadcrumb) return;
    if (document.getElementById('bc-rename-input')) return;

    const currentTitle = currentTaskBreadcrumb.textContent || 'Conversation';
    const input = document.createElement('input');
    input.type = 'text';
    input.id = 'bc-rename-input';
    input.className = 'bc-rename-input';
    input.value = currentTitle;

    currentTaskBreadcrumb.classList.add('hidden');
    if (btnRenameSession) btnRenameSession.classList.add('hidden');
    currentTaskBreadcrumb.parentNode.insertBefore(input, currentTaskBreadcrumb.nextSibling);
    input.focus();
    input.select();

    let finished = false;
    const cleanup = () => {
        if (input.parentNode) input.remove();
        currentTaskBreadcrumb.classList.remove('hidden');
        updateBreadcrumbSessionUI(currentTaskBreadcrumb.textContent);
    };

    const commit = async () => {
        if (finished) return;
        finished = true;
        const nextTitle = input.value.trim();
        if (!nextTitle || nextTitle === currentTitle) {
            cleanup();
            return;
        }
        try {
            const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}`, {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ title: nextTitle })
            });
            if (res.ok) {
                const data = await res.json();
                currentTaskBreadcrumb.textContent = data.title || nextTitle;
                await fetchSessions();
            }
        } catch (_) {}
        cleanup();
    };

    input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            commit();
        } else if (e.key === 'Escape') {
            e.preventDefault();
            finished = true;
            cleanup();
        }
    });
    input.addEventListener('blur', commit);
}

let sseReconnectAttempts = 0;
let sseReconnectTimer = null;

function connectEventStream(sessionId) {
    if (sseReconnectTimer) {
        clearTimeout(sseReconnectTimer);
        sseReconnectTimer = null;
    }
    if (activeEventSource) {
        activeEventSource.close();
        activeEventSource = null;
    }
    activeEventStreamSessionId = sessionId;

    const es = new EventSource(harnessSseUrl(`/api/sessions/${encodeURIComponent(sessionId)}/events`));
    activeEventSource = es;

    es.onopen = () => {
        if (activeEventSource === es) {
            sseReconnectAttempts = 0;
        }
    };

    es.onmessage = (e) => {
        try {
            const evt = JSON.parse(e.data);
            // The server replays the stored history when a stream is opened, so a
            // client that attaches to an already-finished (or approval-blocked)
            // session still sees the Error event and the permission card instead of
            // spinning on its own optimistic flag forever. `__replay` keeps those
            // events from double-rendering what the client already drew.
            if (evt && evt.__replay && evt.event) {
                handleHarnessEvent(evt.event, true);
            } else {
                handleHarnessEvent(evt);
            }
        } catch (_) {}
    };

    es.onerror = async () => {
        if (activeEventSource === es) {
            es.close();
            activeEventSource = null;
            activeEventStreamSessionId = null;
        } else {
            es.close();
            return;
        }

        await fetchSessions();
        const stillSameSession = currentSessionId === sessionId;
        const sess = stillSameSession ? allSessions.find(s => s.id === sessionId) : null;
        if (stillSameSession && sess && sess.status === 'running' && sseReconnectAttempts < 3) {
            sseReconnectAttempts += 1;
            const delayMs = sseReconnectAttempts * 600;
            sseReconnectTimer = setTimeout(() => {
                sseReconnectTimer = null;
                if (currentSessionId === sessionId) {
                    connectEventStream(sessionId);
                }
            }, delayMs);
            return;
        }

        btnSubmit.disabled = false;
        setSessionRunning(false);
        finalizeOrphanedStreamStates();
    };
}

function ensureEventStreamConnected(sessionId) {
    if (!sessionId) return Promise.resolve();
    if (
        activeEventSource &&
        activeEventStreamSessionId === sessionId &&
        activeEventSource.readyState !== EventSource.CLOSED
    ) {
        return Promise.resolve();
    }
    return new Promise((resolve) => {
        connectEventStream(sessionId);
        const es = activeEventSource;
        if (!es) {
            resolve();
            return;
        }
        let settled = false;
        const done = () => {
            if (!settled) {
                settled = true;
                resolve();
            }
        };
        es.addEventListener('open', done, { once: true });
        setTimeout(done, 250);
    });
}

function handleHarnessEvent(event, isReplay = false) {
    if (!event) return;
    const type = event.type;
    const data = event.data || {};

    switch (type) {
        case 'SessionStarted':
            if (!isReplay) setSessionRunning(true);
            break;

        case 'UserMessage': {
            const turnId = Number(data.turn_id);
            if (turnId) activeTurnId = turnId;
            activeReasoningDrawer = null;
            activeMessageBox = null;
            const atts = Array.isArray(data.attachments) ? data.attachments : [];
            if ((data.content || '').trim() || atts.length > 0) {
                renderOrUpgradeUserMessage(turnId || null, data.content || '', atts);
            }
            if (!isReplay) setSessionRunning(true);
            break;
        }

        case 'SystemCommand': {
            const turnId = Number(data.turn_id);
            if (turnId) activeTurnId = turnId;
            activeReasoningDrawer = null;
            activeMessageBox = null;
            renderSystemCommandBanner(data);
            if (!isReplay) setSessionRunning(true);
            break;
        }

        case 'PromptQueued':
            if (!isReplay) {
                const qid = data.queue_id;
                if (qid && !queuedPrompts.some(q => (q.id || q.queue_id) === qid)) {
                    queuedPrompts.push({ id: qid, content: data.content || '' });
                    renderQueuedPromptsTray();
                }
            }
            break;

        case 'PromptDequeued':
            if (!isReplay) {
                const qid = data.queue_id;
                if (qid) {
                    queuedPrompts = queuedPrompts.filter(q => (q.id || q.queue_id) !== qid);
                    renderQueuedPromptsTray();
                }
            }
            break;

        case 'QueueUpdated':
            if (!isReplay) {
                queuedPrompts = Array.isArray(data.queue) ? data.queue : [];
                renderQueuedPromptsTray();
            }
            break;

        case 'SteeringApplied':
            appendSteeringPill(data.content || '');
            break;

        case 'IterationStep':
            activeReasoningDrawer = null;
            activeMessageBox = null;
            break;

        case 'ThoughtDelta':
            appendReasoningDelta(data.delta || '');
            break;

        case 'MessageDelta':
            appendMessageDelta(data.delta || '');
            break;

        case 'PermissionRequested':
            renderPermissionPrompt(data.perm_id, data.tool, data.description, data.args);
            break;

        case 'PermissionDecided':
            updatePermissionPromptDecision(data.perm_id, data.approved, false, data.reason);
            break;

        case 'ToolCallStarted':
            appendToolCall(data.call_id, data.tool, data.args || {});
            break;

        case 'ToolCallFinished':
            updateToolCallResult(data.call_id, data.tool, data.success, data.output, data.duration_ms, data.truncated, isReplay);
            if (!isReplay && isInvisibleSwarmTool(data.tool)) {
                scheduleSwarmStateSync();
            }
            break;

        case 'SubagentSpawned':
            handleSubagentSpawned(data, isReplay);
            break;

        case 'SubagentUpdate':
            handleSubagentUpdate(data);
            break;

        case 'SubagentCompleted':
            handleSubagentCompleted(data);
            break;

        case 'SwarmBoardMessage':
            addSwarmBoardMessage(data, true);
            if (!isReplay) {
                if (inspectorPane && inspectorPane.classList.contains('collapsed') && inspectorActivityDot) {
                    inspectorActivityDot.classList.remove('hidden');
                }
                scheduleSwarmStateSync();
            }
            break;

        case 'ArtifactCreated':
            handleArtifactCreated(data, isReplay);
            break;

        case 'BrowserSnapshot':
            addBrowserSnapshot(data);
            if (!isReplay) openInspector('browser', true);
            break;

        case 'MetricsUpdate':
            if (statCpu && typeof data.cpu_usage_pct === 'number') {
                statCpu.textContent = `${data.cpu_usage_pct.toFixed(1)}% CPU`;
            }
            if (typeof data.rss_memory_mb === 'number') {
                const rssStr = `${data.rss_memory_mb.toFixed(1)} MB`;
                if (statRss) statRss.textContent = rssStr;
                if (settingsStatRss) settingsStatRss.textContent = rssStr;
            }
            if (telemetryPill) {
                const cpuStr = typeof data.cpu_usage_pct === 'number' ? `${data.cpu_usage_pct.toFixed(1)}% CPU` : (statCpu?.textContent || '--');
                const rssStr = typeof data.rss_memory_mb === 'number' ? `${data.rss_memory_mb.toFixed(1)} MB` : (statRss?.textContent || '--');
                const ramStr = statHostRam?.textContent || '--';
                telemetryPill.title = `CPU: ${cpuStr} · Process RSS: ${rssStr} · Host RAM: ${ramStr}`;
            }
            break;

        case 'TokenUsageUpdate':
            updateTokenUsageDisplay(data);
            break;

        case 'MemoryCompacted':
            applyMemoryCompactedStats(data);
            appendMemoryCompactedPill(data);
            break;

        case 'ToolRetryScheduled': {
            const statusPill = document.getElementById(`tool-status-${data.call_id}`);
            if (statusPill) {
                statusPill.className = 'tool-status-pill retrying';
                const toolName = data.tool || 'tool';
                const attempt = data.attempt || 1;
                const maxAttempts = data.max_attempts || 1;
                statusPill.textContent = `↻ Retrying ${toolName} (attempt ${attempt}/${maxAttempts})`;
                if (data.reason) statusPill.title = String(data.reason);
            }
            break;
        }

        case 'Warning':
            appendSystemWarning(data.message || '');
            break;

        case 'Error':
            appendSystemError(data.message || '');
            finalizeOrphanedStreamStates();
            // A terminal Error means the turn is over, so any pending approval is
            // dead with it. Without this the top bar kept claiming "N awaiting
            // approval" for a prompt the agent had abandoned, and its Approve click
            // 404'd against an already-finished session.
            clearAllPendingPermissions();
            if (!isReplay) {
                btnSubmit.disabled = false;
                setSessionRunning(false);
            }
            break;

        case 'SessionCompleted':
            if (!isReplay) {
                setSessionRunning(false);
            }
            // Terminal either way (including a user Stop): any pending approval is
            // dead, so clear the badge rather than leaving a prompt the agent has
            // abandoned on screen claiming it is still awaiting approval.
            clearAllPendingPermissions();
            finishSession(data.final_answer || '', data.total_steps || 0, data.duration_secs || 0, isReplay);
            break;
    }

    if (!isReplay) scrollToBottom();
}

function getDistanceFromBottom() {
    if (!streamScrollArea) return 0;
    return Math.max(0, streamScrollArea.scrollHeight - streamScrollArea.scrollTop - streamScrollArea.clientHeight);
}

function updateScrollBottomButton() {
    if (!btnScrollBottom) return;
    const dist = getDistanceFromBottom();
    const show = !shouldAutoScroll && dist > 24;
    btnScrollBottom.classList.toggle('hidden', !show);
}

function syncShouldAutoScrollFlag(val) {
    shouldAutoScroll = !!val;
    if (streamContainer) streamContainer.shouldAutoScroll = shouldAutoScroll;
    updateScrollBottomButton();
}

function scrollToBottom(force = false) {
    if (force === true) {
        syncShouldAutoScrollFlag(true);
    }
    if (force === false && !shouldAutoScroll) {
        updateScrollBottomButton();
        return;
    }
    if (streamScrollArea) {
        isProgrammaticScroll = true;
        streamScrollArea.scrollTop = streamScrollArea.scrollHeight;
        requestAnimationFrame(() => { isProgrammaticScroll = false; });
    }
    updateScrollBottomButton();
}

function renderSystemCommandBanner(data) {
    if (!streamContainer || !data) return;
    const turnId = Number(data.turn_id);
    if (turnId) {
        const existing = streamContainer.querySelector(`.system-command-banner[data-turn-id="${turnId}"]`);
        if (existing) return existing;
    }

    const cmd = String(data.command || '').replace(/^\/+/, '').trim();
    const args = String(data.args || '').trim();
    const label = String(data.label || (cmd ? `/${cmd}` : 'System Directive')).trim();
    const iconKey = data.icon || cmd || 'skill';

    const banner = document.createElement('div');
    banner.className = 'system-command-banner';
    if (turnId) banner.dataset.turnId = String(turnId);
    banner.innerHTML = `
        <div class="system-command-left">
            <span class="system-command-icon">${getCommandSvgIcon(iconKey, 15)}</span>
            <div class="system-command-text">
                <span class="system-command-name">/${escapeHtml(cmd)}</span>
                <span class="system-command-label">${escapeHtml(label)}</span>
                ${args ? `<span class="system-command-args">${escapeHtml(args)}</span>` : ''}
            </div>
        </div>
        <span class="system-command-tag">Directive</span>
    `;
    streamContainer.appendChild(banner);
    return banner;
}

// Reasoning Stream (Accordion)
function appendReasoningDelta(delta) {
    if (!activeReasoningDrawer) {
        activeReasoningDrawer = document.createElement('details');
        activeReasoningDrawer.className = 'reasoning-accordion';
        activeReasoningDrawer.open = (activeDisplayVerbosity === 'verbose');
        activeReasoningDrawer.innerHTML = `
            <summary class="reasoning-summary">
                <div class="reasoning-summary-left">
                    <span class="pulse-dot"></span>
                    <span class="reasoning-title-text">Reasoning Trace</span>
                </div>
                <span class="reasoning-toggle-hint">collapse / expand</span>
            </summary>
            <div class="reasoning-body"></div>
        `;
        const summaryEl = activeReasoningDrawer.querySelector('.reasoning-summary');
        const drawerRef = activeReasoningDrawer;
        if (summaryEl) {
            summaryEl.addEventListener('click', () => {
                drawerRef.dataset.userToggled = '1';
            });
        }
        streamContainer.appendChild(activeReasoningDrawer);
    }
    const body = activeReasoningDrawer.querySelector('.reasoning-body');
    body.textContent += delta;

    const count = Math.round(body.textContent.length / 4);
    const titleText = activeReasoningDrawer.querySelector('.reasoning-title-text');
    if (titleText) titleText.textContent = `Reasoning Trace (~${count} tokens)`;
}

// Assistant Response Stream
function appendMessageDelta(delta) {
    if (!activeMessageBox) {
        activeMessageBox = document.createElement('div');
        activeMessageBox.className = 'message-bubble assistant-msg-bubble';
        if (activeTurnId) activeMessageBox.dataset.turnId = String(activeTurnId);
        activeMessageBox.innerHTML = `
            <div class="assistant-msg-content" data-raw=""></div>
            <div class="msg-action-bar assistant-msg-actions">
                <button type="button" class="msg-action-btn" data-action="copy" title="Copy markdown">${getCopySvgIcon()} Copy</button>
                <button type="button" class="msg-action-btn" data-action="retry" title="Undo turn and regenerate response">🔄 Retry</button>
                <button type="button" class="msg-action-btn" data-action="fork" title="Fork conversation after this response">⑂ Fork</button>
                <button type="button" class="msg-action-btn" data-action="undo" title="Undo this turn">↩ Undo</button>
            </div>
        `;
        wireAssistantBubbleActions(activeMessageBox);
        streamContainer.appendChild(activeMessageBox);
    }
    const content = activeMessageBox.querySelector('.assistant-msg-content');
    const raw = (content.getAttribute('data-raw') || '') + delta;
    content.setAttribute('data-raw', raw);
    content.innerHTML = renderMarkdown(raw);
}

// ==========================================================================
// Interactive Tool Execution Cards (Bash, Diff, Browser UI, Generic)
// ==========================================================================
const INVISIBLE_SWARM_TOOLS = new Set([
    'invoke_subagent',
    'swarm_board',
    'swarm',
    'swarm_wait',
    'swarm_execute',
    'swarm_status'
]);

function isInvisibleSwarmTool(toolName) {
    return INVISIBLE_SWARM_TOOLS.has(String(toolName || '').trim().toLowerCase());
}

function appendToolCall(callId, tool, args) {
    const safeArgs = args && typeof args === 'object' ? args : {};
    toolCallsMap.set(callId, { tool, args: safeArgs, success: null, output: '', copyPayload: '' });

    if (isInvisibleSwarmTool(tool)) {
        return null;
    }

    const existingCard = document.getElementById(`tool-${callId}`);
    if (existingCard) {
        return existingCard;
    }

    const card = document.createElement('div');
    card.id = `tool-${callId}`;
    card.className = `tool-card${activeDisplayVerbosity === 'verbose' ? ' expanded' : ''}`;

    let argSummary = '';
    let bodyHtml = '';

    if (tool === 'bash') {
        argSummary = safeArgs.command || '';
        bodyHtml = `<div class="tool-command-strip"><span class="tool-cmd-prompt">$</span> ${escapeHtml(safeArgs.command || '')}</div>`;
    } else if (tool === 'edit_file' || tool === 'write_file') {
        const filePath = safeArgs.path || safeArgs.file_path || '';
        argSummary = filePath;
        bodyHtml = buildDiffPreviewHtml(tool, safeArgs);
    } else if (tool === 'browser_ui') {
        argSummary = `${safeArgs.action || 'navigate'} → ${safeArgs.url || ''}`;
        bodyHtml = `<div class="tool-browser-preview" id="tool-browser-${escapeHtml(callId)}">
            <div class="browser-preview-meta">
                <span class="browser-preview-url">${escapeHtml(safeArgs.url || '')}</span>
            </div>
        </div>`;
    } else if (tool === 'grep_search') {
        const pattern = safeArgs.pattern || safeArgs.query || '';
        const scope = safeArgs.path || safeArgs.dir || '.';
        argSummary = `/${pattern}/ in ${scope}`;
    } else if (tool === 'list_dir') {
        argSummary = safeArgs.path || '.';
    } else if (tool === 'web_search') {
        argSummary = `"${safeArgs.query || ''}"`;
    } else if (tool === 'web_fetch') {
        argSummary = safeArgs.url || '';
    } else if (tool === 'read_file') {
        const filePath = safeArgs.path || safeArgs.file_path || '';
        const rangeHint = (safeArgs.start_line || safeArgs.end_line)
            ? `:${safeArgs.start_line || 1}-${safeArgs.end_line || 'end'}`
            : '';
        argSummary = `${filePath}${rangeHint}`;
    } else {
        argSummary = summarizeGenericArgs(safeArgs);
    }

    const formattedArgsJson = JSON.stringify(safeArgs, null, 2);
    const argsBytesLabel = formatBytes(formattedArgsJson.length);

    card.innerHTML = `
        <div class="tool-card-header" role="button" tabindex="0" aria-expanded="${activeDisplayVerbosity === 'verbose' ? 'true' : 'false'}" title="Click to toggle parameters and output">
            <div class="tool-card-title-group">
                <span class="tool-card-icon">${getToolSvgIcon(tool)}</span>
                <span class="tool-kind-badge">${escapeHtml(tool)}</span>
                <span class="tool-arg-summary" title="${escapeHtml(argSummary)}">${escapeHtml(compactPreviewText(argSummary, 96))}</span>
                <span class="tool-result-inline hidden" id="tool-inline-${escapeHtml(callId)}"></span>
            </div>
            <div class="tool-card-meta">
                <span class="tool-status-pill running" id="tool-status-${escapeHtml(callId)}">Running...</span>
                <span class="tool-metric-badge" id="tool-metric-${escapeHtml(callId)}">${escapeHtml(argsBytesLabel)}</span>
                <button type="button" class="code-btn-action tool-card-copy-in" title="Copy tool input" data-copy-tool="${escapeHtml(callId)}">${getCopySvgIcon()}</button>
                <span class="tool-card-chevron">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>
                </span>
            </div>
        </div>
        <div class="tool-card-details">
            <div class="tool-input-section">
                <div class="tool-detail-label">Parameters</div>
                <pre class="tool-args-pre">${escapeHtml(formattedArgsJson)}</pre>
            </div>
            ${bodyHtml}
            <div class="tool-result-slot" id="tool-result-${escapeHtml(callId)}"></div>
        </div>
    `;

    const headerEl = card.querySelector('.tool-card-header');
    const toggleCardExpanded = () => {
        const nextExpanded = card.classList.toggle('expanded');
        card.dataset.userToggled = '1';
        if (headerEl) headerEl.setAttribute('aria-expanded', nextExpanded ? 'true' : 'false');
    };
    if (headerEl) {
        headerEl.addEventListener('click', (e) => {
            if (e.target && e.target.closest('button, a')) return;
            toggleCardExpanded();
        });
        headerEl.addEventListener('keydown', (e) => {
            if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                toggleCardExpanded();
            }
        });
    }

    const copyBtn = card.querySelector('[data-copy-tool]');
    if (copyBtn) {
        copyBtn.addEventListener('click', async (e) => {
            e.stopPropagation();
            if (copyBtn.dataset.copying === '1') return;
            const payload = tool === 'bash' ? (safeArgs.command || '') : formattedArgsJson;
            if (await copyTextToClipboard(payload)) {
                copyBtn.dataset.copying = '1';
                copyBtn.textContent = '✓';
                setTimeout(() => {
                    copyBtn.innerHTML = getCopySvgIcon();
                    delete copyBtn.dataset.copying;
                }, 1400);
            }
        });
    }

    streamContainer.appendChild(card);
    return card;
}

function summarizeGenericArgs(args) {
    if (!args || typeof args !== 'object') return '';
    const keys = Object.keys(args);
    if (keys.length === 0) return '';
    return keys.slice(0, 3).map(k => {
        const v = typeof args[k] === 'string' ? args[k] : JSON.stringify(args[k]);
        const shortV = String(v).length > 48 ? String(v).slice(0, 45) + '...' : String(v);
        return `${k}: ${shortV}`;
    }).join(' · ');
}

function buildDiffPreviewHtml(tool, args) {
    if (tool === 'edit_file') {
        const oldStr = args.target || args.old_str || args.old_string || '';
        const newStr = args.replacement || args.new_str || args.new_string || '';
        const oldLines = oldStr ? oldStr.split('\n') : [];
        const newLines = newStr ? newStr.split('\n') : [];
        const linesHtml = [
            ...oldLines.slice(0, 40).map(l => `<div class="diff-line del">- ${escapeHtml(l)}</div>`),
            ...newLines.slice(0, 60).map(l => `<div class="diff-line add">+ ${escapeHtml(l)}</div>`)
        ].join('');
        return `
            <details class="tool-output-details" open>
                <summary class="tool-output-summary">
                    <span class="diff-stats">
                        <span>Diff preview</span>
                        <span class="diff-added">+${newLines.length}</span>
                        <span class="diff-removed">-${oldLines.length}</span>
                    </span>
                    <span>toggle diff</span>
                </summary>
                <div class="diff-code-container">${linesHtml || '<div class="diff-line ctx">(no inline diff fields)</div>'}</div>
            </details>
        `;
    }

    const content = args.content || '';
    const lines = content ? content.split('\n') : [];
    const linesHtml = lines.slice(0, 60).map(l => `<div class="diff-line add">+ ${escapeHtml(l)}</div>`).join('');
    const truncatedHint = lines.length > 60 ? `<div class="diff-line ctx">... (${lines.length - 60} more lines)</div>` : '';
    return `
        <details class="tool-output-details">
            <summary class="tool-output-summary">
                <span class="diff-stats">
                    <span>File write</span>
                    <span class="diff-added">+${lines.length} lines</span>
                </span>
                <span>review content</span>
            </summary>
            <div class="diff-code-container">${linesHtml}${truncatedHint}</div>
        </details>
    `;
}

function parseBrowserUiOutput(output) {
    if (!output) return null;
    const fenceMatch = output.match(/```json\s*([\s\S]*?)```/i);
    if (fenceMatch) {
        try {
            return JSON.parse(fenceMatch[1].trim());
        } catch (_) {}
    }
    try {
        const trimmed = output.trim();
        if (trimmed.startsWith('{')) return JSON.parse(trimmed);
    } catch (_) {}
    return null;
}

function parseBashResult(output, success) {
    const raw = String(output || '');
    const trimmed = raw.trim();
    if (!trimmed || /^\(Command completed with exit code 0 and no output\)$/i.test(trimmed)) {
        return { exitCode: success ? 0 : 1, stdout: '', stderr: '', empty: true };
    }

    const failMatch = raw.match(/^Command failed with exit code (-?\d+)\r?\nStdout:\r?\n([\s\S]*?)\r?\nStderr:\r?\n([\s\S]*)$/);
    if (failMatch) {
        return {
            exitCode: Number(failMatch[1]),
            stdout: failMatch[2].replace(/\r?\n$/, ''),
            stderr: failMatch[3].replace(/\r?\n$/, ''),
            empty: !failMatch[2].trim() && !failMatch[3].trim()
        };
    }

    const stderrSplit = raw.split(/\r?\n--- stderr ---\r?\n/);
    if (stderrSplit.length > 1) {
        return {
            exitCode: success ? 0 : 1,
            stdout: stderrSplit[0].replace(/\r?\n$/, ''),
            stderr: stderrSplit.slice(1).join('\n--- stderr ---\n').replace(/\r?\n$/, ''),
            empty: !stderrSplit[0].trim() && !stderrSplit.slice(1).join('').trim()
        };
    }

    return {
        exitCode: success ? 0 : 1,
        stdout: success ? raw : '',
        stderr: success ? '' : raw,
        empty: !trimmed
    };
}

function parseListDirResult(output) {
    const lines = String(output || '').split(/\r?\n/).map(l => l.trim()).filter(Boolean);
    if (lines.length === 0) return null;
    const entries = [];
    for (const line of lines) {
        if (line.startsWith('[DIR]')) {
            const name = line.slice(5).trim().replace(/\/+$/, '');
            entries.push({ kind: 'dir', name, bytes: null });
        } else if (line.startsWith('[FILE]')) {
            const rest = line.slice(6).trim();
            const m = rest.match(/^(.*?)\s+\((\d+)\s+bytes?\)$/i);
            if (m) {
                entries.push({ kind: 'file', name: m[1].trim(), bytes: Number(m[2]) });
            } else {
                entries.push({ kind: 'file', name: rest, bytes: null });
            }
        } else {
            return null;
        }
    }
    return {
        entries,
        dirCount: entries.filter(e => e.kind === 'dir').length,
        fileCount: entries.filter(e => e.kind === 'file').length
    };
}

function parseGrepResult(output) {
    const raw = String(output || '').trim();
    if (!raw) return null;
    if (/^Pattern\s+['"].*['"]\s+not found/i.test(raw)) {
        return { notFound: true, message: raw, matches: [], fileCount: 0 };
    }
    const lines = raw.split(/\r?\n/).filter(Boolean);
    const matches = [];
    const files = new Set();
    for (const line of lines) {
        const m = line.match(/^(.+?):(\d+):\s?(.*)$/);
        if (!m) {
            if (matches.length === 0) return null;
            matches.push({ file: '', line: '', snippet: line });
            continue;
        }
        files.add(m[1]);
        matches.push({ file: m[1], line: m[2], snippet: m[3] });
    }
    if (matches.length === 0) return null;
    return { notFound: false, matches, fileCount: files.size };
}

function parseReadFileResult(output) {
    const raw = String(output || '');
    const trimmed = raw.trim();
    if (!trimmed) return { empty: true, lines: [], numbered: false, codeText: '' };
    if (/^\(Empty:/i.test(trimmed)) {
        return { empty: true, message: trimmed, lines: [], numbered: false, codeText: '' };
    }
    const rawLines = raw.split(/\r?\n/);
    if (rawLines.length > 0 && rawLines[rawLines.length - 1] === '') rawLines.pop();
    const parsedLines = [];
    const codeRows = [];
    let allNumbered = rawLines.length > 0;
    for (const line of rawLines) {
        const m = line.match(/^\s*(\d+)\s\|\s?(.*)$/);
        if (m) {
            parsedLines.push({ num: m[1], text: m[2] });
            codeRows.push(m[2]);
        } else {
            allNumbered = false;
            parsedLines.push({ num: '', text: line });
            codeRows.push(line);
        }
    }
    return {
        empty: false,
        lines: parsedLines,
        numbered: allNumbered,
        codeText: allNumbered ? codeRows.join('\n') : raw
    };
}

function tryParseJsonOutput(output) {
    const trimmed = String(output || '').trim();
    if (!trimmed || (trimmed[0] !== '{' && trimmed[0] !== '[')) return null;
    try {
        return JSON.parse(trimmed);
    } catch (_) {
        return null;
    }
}

function summarizeToolResultInline(tool, success, output, meta = {}) {
    const raw = String(output || '').trim();
    if (tool === 'bash') {
        const b = meta.bash || parseBashResult(output, success);
        if (b.empty) return b.exitCode === 0 ? 'exit 0 · empty' : `exit ${b.exitCode}`;
        const combined = [b.stdout, b.stderr].filter(Boolean).join('\n');
        const lc = combined ? combined.split(/\r?\n/).length : 0;
        return `exit ${b.exitCode}${lc > 0 ? ` · ${lc}L` : ''}`;
    }
    if (tool === 'read_file') {
        const rf = meta.readFile || parseReadFileResult(output);
        if (rf.empty) return 'empty';
        return `${rf.lines.length}L`;
    }
    if (tool === 'list_dir') {
        const ld = meta.listDir || parseListDirResult(output);
        if (ld) {
            const parts = [];
            if (ld.dirCount > 0) parts.push(`${ld.dirCount}d`);
            if (ld.fileCount > 0) parts.push(`${ld.fileCount}f`);
            return parts.length > 0 ? parts.join(' ') : '0 items';
        }
    }
    if (tool === 'grep_search') {
        const gr = meta.grep || parseGrepResult(output);
        if (gr) {
            return gr.notFound ? '0 matches' : `${gr.matches.length} match${gr.matches.length === 1 ? '' : 'es'} · ${gr.fileCount}f`;
        }
    }
    if (tool === 'browser_ui' && meta.browser) {
        const errCount = Array.isArray(meta.browser.console_errors) ? meta.browser.console_errors.length : 0;
        const code = meta.browser.http_status ? `${meta.browser.http_status}` : '';
        return [code, `${errCount} err`].filter(Boolean).join(' · ');
    }
    if (tool === 'edit_file' || tool === 'write_file') {
        if (!success && raw) return compactPreviewText(raw, 40);
        const args = meta.args || {};
        if (tool === 'edit_file') {
            const oldL = (args.target || args.old_str || args.old_string || '').split('\n').filter(Boolean).length;
            const newL = (args.replacement || args.new_str || args.new_string || '').split('\n').filter(Boolean).length;
            if (oldL || newL) return `+${newL} -${oldL}`;
        } else if (tool === 'write_file' && raw) {
            const bytesM = raw.match(/(\d+)\s+bytes/i);
            if (bytesM) return formatBytes(Number(bytesM[1]));
        }
    }
    if (!raw) return success ? 'done' : 'failed';
    const lineCount = raw.split(/\r?\n/).length;
    if (lineCount > 1) return `${lineCount}L`;
    return compactPreviewText(raw, 44);
}

function buildToolCopyOutputButton(callId, label = 'Copy') {
    return `<button type="button" class="code-btn-action tool-output-copy-btn" data-copy-tool-output="${escapeHtml(callId)}" title="Copy tool output">${getCopySvgIcon()} <span>${escapeHtml(label)}</span></button>`;
}

function buildStructuredToolOutputHtml(callId, tool, success, output, truncated, manageToolsBadgeHtml, meta = {}) {
    const raw = String(output || '');
    const truncTag = truncated ? ' · truncated' : '';

    if (tool === 'bash') {
        const b = meta.bash || parseBashResult(raw, success);
        const exitBadge = `<span class="tool-exit-badge ${b.exitCode === 0 ? 'ok' : 'err'}">exit ${b.exitCode}</span>`;
        if (b.empty) {
            return `
                ${manageToolsBadgeHtml}
                <div class="tool-output-header tool-output-header-terminal">
                    <div class="tool-output-header-left">
                        ${exitBadge}
                        <span class="tool-output-submeta">No terminal output${truncTag}</span>
                    </div>
                </div>
            `;
        }
        const outLines = b.stdout ? b.stdout.split(/\r?\n/).length : 0;
        const errLines = b.stderr ? b.stderr.split(/\r?\n/).length : 0;
        const totalLines = outLines + errLines;
        let panesHtml = '';
        if (b.stdout) {
            panesHtml += `
                ${b.stderr ? '<div class="tool-stream-sublabel">Stdout</div>' : ''}
                <pre class="tool-output-pre tool-terminal-stdout">${escapeHtml(b.stdout)}</pre>
            `;
        }
        if (b.stderr) {
            panesHtml += `
                <div class="tool-stream-sublabel stderr">Stderr</div>
                <pre class="tool-output-pre tool-terminal-stderr">${escapeHtml(b.stderr)}</pre>
            `;
        }
        return `
            ${manageToolsBadgeHtml}
            <div class="tool-output-header tool-output-header-terminal">
                <div class="tool-output-header-left">
                    ${exitBadge}
                    <span class="tool-output-submeta">${totalLines} line${totalLines === 1 ? '' : 's'}${truncTag}</span>
                </div>
                ${buildToolCopyOutputButton(callId)}
            </div>
            ${panesHtml}
        `;
    }

    if (tool === 'read_file' && success) {
        const rf = meta.readFile || parseReadFileResult(raw);
        if (rf.empty) {
            return `
                ${manageToolsBadgeHtml}
                <div class="tool-output-header"><span>${escapeHtml(rf.message || 'Empty file')}</span></div>
            `;
        }
        const maxRows = 400;
        const sliced = rf.lines.slice(0, maxRows);
        const rowsHtml = sliced.map(row => `
            <div class="tool-code-row">
                ${rf.numbered ? `<span class="tool-code-ln">${escapeHtml(row.num)}</span>` : ''}
                <span class="tool-code-text">${escapeHtml(row.text)}</span>
            </div>
        `).join('');
        const extraHint = rf.lines.length > maxRows
            ? `<div class="tool-viewer-more">Showing first ${maxRows} of ${rf.lines.length} lines (use Copy for full text)</div>`
            : '';
        return `
            ${manageToolsBadgeHtml}
            <div class="tool-output-header">
                <span>File Content · ${rf.lines.length} line${rf.lines.length === 1 ? '' : 's'}${truncTag}</span>
                ${buildToolCopyOutputButton(callId)}
            </div>
            <div class="tool-code-viewer">${rowsHtml}${extraHint}</div>
        `;
    }

    if (tool === 'list_dir' && success) {
        const ld = meta.listDir || parseListDirResult(raw);
        if (ld) {
            const folderSvg = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>';
            const fileSvg = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>';
            const rowsHtml = ld.entries.slice(0, 200).map(e => `
                <div class="tool-dir-row ${e.kind}">
                    <span class="tool-dir-icon">${e.kind === 'dir' ? folderSvg : fileSvg}</span>
                    <span class="tool-dir-name">${escapeHtml(e.name)}${e.kind === 'dir' ? '/' : ''}</span>
                    <span class="tool-dir-size">${e.bytes !== null ? escapeHtml(formatBytes(e.bytes)) : (e.kind === 'dir' ? 'dir' : '')}</span>
                </div>
            `).join('');
            return `
                ${manageToolsBadgeHtml}
                <div class="tool-output-header">
                    <span>Directory · ${ld.dirCount} dir${ld.dirCount === 1 ? '' : 's'} · ${ld.fileCount} file${ld.fileCount === 1 ? '' : 's'}${truncTag}</span>
                    ${buildToolCopyOutputButton(callId)}
                </div>
                <div class="tool-dir-list">${rowsHtml}</div>
            `;
        }
    }

    if (tool === 'grep_search' && success) {
        const gr = meta.grep || parseGrepResult(raw);
        if (gr) {
            if (gr.notFound) {
                return `
                    ${manageToolsBadgeHtml}
                    <div class="tool-output-header"><span>Search Results · 0 matches</span></div>
                    <div class="tool-empty-result">${escapeHtml(gr.message)}</div>
                `;
            }
            const rowsHtml = gr.matches.slice(0, 200).map(m => `
                <div class="tool-grep-row">
                    ${m.file ? `<span class="tool-grep-loc" title="${escapeHtml(m.file)}:${escapeHtml(m.line)}"><span class="tool-grep-file">${escapeHtml(m.file)}</span><span class="tool-grep-ln">:${escapeHtml(m.line)}</span></span>` : ''}
                    <span class="tool-grep-snippet">${escapeHtml(m.snippet)}</span>
                </div>
            `).join('');
            const extraHint = gr.matches.length > 200
                ? `<div class="tool-viewer-more">Showing first 200 of ${gr.matches.length} matches</div>`
                : '';
            return `
                ${manageToolsBadgeHtml}
                <div class="tool-output-header">
                    <span>${gr.matches.length} match${gr.matches.length === 1 ? '' : 'es'} across ${gr.fileCount} file${gr.fileCount === 1 ? '' : 's'}${truncTag}</span>
                    ${buildToolCopyOutputButton(callId)}
                </div>
                <div class="tool-grep-list">${rowsHtml}${extraHint}</div>
            `;
        }
    }

    if (tool === 'browser_ui' && meta.browser && raw.trim()) {
        const linesCount = raw.split('\n').length;
        return `
            ${manageToolsBadgeHtml}
            <details class="tool-output-details">
                <summary class="tool-output-summary">
                    <span>Raw browser_ui payload (${linesCount} line${linesCount === 1 ? '' : 's'}${truncTag})</span>
                    ${buildToolCopyOutputButton(callId)}
                </summary>
                <pre class="tool-output-pre">${escapeHtml(raw)}</pre>
            </details>
        `;
    }

    if (raw.trim()) {
        const jsonVal = tryParseJsonOutput(raw);
        const formatted = jsonVal !== null ? JSON.stringify(jsonVal, null, 2) : raw;
        const linesCount = formatted.split('\n').length;
        const kindLabel = jsonVal !== null ? 'JSON Output' : 'Output';
        return `
            ${manageToolsBadgeHtml}
            <div class="tool-output-header">
                <span>${kindLabel} (${linesCount} line${linesCount === 1 ? '' : 's'}${truncTag})</span>
                ${buildToolCopyOutputButton(callId)}
            </div>
            <pre class="tool-output-pre">${escapeHtml(formatted)}</pre>
        `;
    }

    return manageToolsBadgeHtml || '';
}

function updateToolCallResult(callId, tool, success, output, durationMs, truncated, isReplay = false) {
    const existingRecord = toolCallsMap.get(callId) || { tool, args: {}, success: null };
    existingRecord.success = !!success;
    existingRecord.output = output || '';
    existingRecord.durationMs = durationMs;
    existingRecord.truncated = !!truncated;

    if (isInvisibleSwarmTool(tool)) {
        toolCallsMap.set(callId, existingRecord);
        return;
    }

    const parsedMeta = { args: existingRecord.args };
    if (tool === 'bash') parsedMeta.bash = parseBashResult(output, success);
    else if (tool === 'list_dir') parsedMeta.listDir = parseListDirResult(output);
    else if (tool === 'grep_search') parsedMeta.grep = parseGrepResult(output);
    else if (tool === 'read_file') {
        parsedMeta.readFile = parseReadFileResult(output);
        if (parsedMeta.readFile && parsedMeta.readFile.codeText) {
            existingRecord.copyPayload = parsedMeta.readFile.codeText;
        }
    } else if (tool === 'browser_ui') {
        parsedMeta.browser = parseBrowserUiOutput(output);
    }
    if (!existingRecord.copyPayload) existingRecord.copyPayload = output || '';
    toolCallsMap.set(callId, existingRecord);

    const card = document.getElementById(`tool-${callId}`);
    if (card) {
        card.dataset.success = success ? 'true' : 'false';
        if (card.dataset.userToggled !== '1') {
            const shouldExpand = activeDisplayVerbosity === 'verbose'
                || (activeDisplayVerbosity === 'balanced' && !success);
            card.classList.toggle('expanded', shouldExpand);
            const hdr = card.querySelector('.tool-card-header');
            if (hdr) hdr.setAttribute('aria-expanded', shouldExpand ? 'true' : 'false');
        }
    }

    const statusPill = document.getElementById(`tool-status-${callId}`);
    if (statusPill) {
        statusPill.className = `tool-status-pill ${success ? 'success' : 'error'}`;
        if (tool === 'bash' && parsedMeta.bash && !success && Number.isFinite(parsedMeta.bash.exitCode)) {
            statusPill.textContent = `✕ Exit ${parsedMeta.bash.exitCode}`;
        } else {
            statusPill.textContent = success ? '✓ Done' : '✕ Failed';
        }
    }

    const inlineEl = document.getElementById(`tool-inline-${callId}`);
    if (inlineEl) {
        const inlineText = summarizeToolResultInline(tool, success, output, parsedMeta);
        if (inlineText) {
            inlineEl.textContent = inlineText;
            inlineEl.classList.remove('hidden', 'ok', 'err');
            inlineEl.classList.add(success ? 'ok' : 'err');
        }
    }

    const metricBadge = document.getElementById(`tool-metric-${callId}`);
    if (metricBadge) {
        const parts = [];
        if (typeof durationMs === 'number') parts.push(`${durationMs}ms`);
        if (output) parts.push(formatBytes(output.length));
        if (parts.length > 0) metricBadge.textContent = parts.join(' · ');
    }

    const resultSlot = document.getElementById(`tool-result-${callId}`);
    if (!resultSlot) return;

    if (tool === 'browser_ui') {
        const parsed = parsedMeta.browser;
        const previewContainer = document.getElementById(`tool-browser-${callId}`);
        if (parsed && previewContainer) {
            const errCount = Array.isArray(parsed.console_errors) ? parsed.console_errors.length : 0;
            const shotUrl = parsed.screenshot_url ? appendSessionQuery(parsed.screenshot_url) : null;
            const statusChip = parsed.http_status ? `<span class="tool-kind-badge">HTTP ${escapeHtml(String(parsed.http_status))}</span>` : '';
            const modeChip = parsed.mode ? `<span class="inspector-sub-pill">${escapeHtml(parsed.mode)}</span>` : '';

            previewContainer.innerHTML = `
                <div class="browser-preview-meta">
                    ${statusChip}
                    ${modeChip}
                    <span class="console-badge ${errCount > 0 ? 'err' : 'ok'}">${errCount} console error${errCount === 1 ? '' : 's'}</span>
                    <span class="browser-preview-url" title="${escapeHtml(parsed.url || '')}">${escapeHtml(parsed.title || parsed.url || '')}</span>
                </div>
                ${parsed.dom_summary ? `<div class="browser-dom-summary">${escapeHtml(parsed.dom_summary)}</div>` : ''}
                ${shotUrl ? `<img src="${escapeHtml(shotUrl)}" alt="browser_ui screenshot" class="tool-browser-thumb">` : ''}
            `;

            const thumb = previewContainer.querySelector('.tool-browser-thumb');
            if (thumb && shotUrl) {
                thumb.addEventListener('click', () => openImageLightbox(shotUrl, parsed.title || parsed.url));
            }
        }
    }

    let manageToolsBadgeHtml = '';
    if (tool === 'manage_tools' && success && output) {
        let parsedManage = null;
        try {
            const trimmed = output.trim();
            if (trimmed.startsWith('{')) parsedManage = JSON.parse(trimmed);
        } catch (_) {}
        const statusVal = parsedManage && parsedManage.status ? String(parsedManage.status) : '';
        const isMutation = statusVal === 'created' || statusVal === 'removed'
            || output.includes('"created"') || output.includes('"removed"');
        if (isMutation) {
            const targetName = parsedManage && parsedManage.name ? ` \`${parsedManage.name}\`` : '';
            const verb = statusVal === 'removed' || output.includes('"removed"') ? 'removed' : 'created';
            manageToolsBadgeHtml = `<div class="manage-tools-notice-badge">Registry updated: custom tool${escapeHtml(targetName)} ${escapeHtml(verb)}</div>`;
            if (!isReplay) {
                loadTools();
            }
        }
    }

    const structuredHtml = buildStructuredToolOutputHtml(
        callId,
        tool,
        success,
        output,
        truncated,
        manageToolsBadgeHtml,
        parsedMeta
    );
    if (structuredHtml) {
        resultSlot.innerHTML = structuredHtml;
        const copyOutBtn = resultSlot.querySelector('[data-copy-tool-output]');
        if (copyOutBtn) {
            copyOutBtn.addEventListener('click', async (e) => {
                e.preventDefault();
                e.stopPropagation();
                if (copyOutBtn.dataset.copying === '1') return;
                const rec = toolCallsMap.get(callId);
                const payload = rec ? (rec.copyPayload || rec.output || '') : (output || '');
                if (await copyTextToClipboard(payload)) {
                    copyOutBtn.dataset.copying = '1';
                    const orig = copyOutBtn.innerHTML;
                    copyOutBtn.textContent = '✓ Copied';
                    setTimeout(() => {
                        copyOutBtn.innerHTML = orig;
                        delete copyOutBtn.dataset.copying;
                    }, 1400);
                }
            });
        }
    }
}

// ==========================================================================
// Background Swarm Subagent Tracking & Artifact Stream Cards
// ==========================================================================
function handleSubagentSpawned(data, isReplay) {
    const sub = {
        id: data.subagent_id,
        role: data.role || 'worker',
        task: compactPreviewText(data.task || '', 120),
        parent_id: data.parent_id || null,
        job_tag: data.job_tag || null,
        specialist: data.specialist || null,
        status: 'running',
        summary: 'Spawned in background...',
        duration_secs: null,
        traces: []
    };
    sessionSubagents.set(sub.id, sub);

    renderInspectorSwarm();
    updateTaskRunningIndicator();
    if (!isReplay) {
        if (inspectorPane && inspectorPane.classList.contains('collapsed') && inspectorActivityDot) {
            inspectorActivityDot.classList.remove('hidden');
        }
        scheduleSwarmStateSync();
    }
}

function handleSubagentUpdate(data) {
    let sub = sessionSubagents.get(data.subagent_id);
    if (!sub) {
        sub = {
            id: data.subagent_id,
            role: data.role || 'worker',
            task: '',
            parent_id: null,
            job_tag: data.job_tag || null,
            specialist: data.specialist || null,
            status: data.status || 'running',
            summary: '',
            duration_secs: null,
            traces: []
        };
        sessionSubagents.set(sub.id, sub);
    }

    if (data.status) sub.status = data.status;
    if (data.job_tag) sub.job_tag = data.job_tag;
    if (data.specialist) sub.specialist = data.specialist;
    if (data.content) {
        sub.summary = compactPreviewText(data.content, 140);
        sub.traces.push({
            kind: data.delta_kind || 'update',
            content: sub.summary
        });
        if (sub.traces.length > 40) sub.traces.shift();
    }

    renderInspectorSwarm();
    updateTaskRunningIndicator();
}

function handleSubagentCompleted(data) {
    let sub = sessionSubagents.get(data.subagent_id);
    if (!sub) {
        sub = {
            id: data.subagent_id,
            role: data.role || 'worker',
            task: '',
            parent_id: null,
            job_tag: data.job_tag || null,
            specialist: data.specialist || null,
            status: data.success ? 'completed' : 'failed',
            summary: compactPreviewText(data.summary || '', 140),
            duration_secs: data.duration_secs,
            traces: []
        };
        sessionSubagents.set(sub.id, sub);
    } else {
        sub.status = data.success ? 'completed' : 'failed';
        if (data.job_tag) sub.job_tag = data.job_tag;
        if (data.specialist) sub.specialist = data.specialist;
        if (data.summary) sub.summary = compactPreviewText(data.summary, 140);
        sub.duration_secs = data.duration_secs;
    }

    renderInspectorSwarm();
    updateTaskRunningIndicator();
    scheduleSwarmStateSync();
}

function handleArtifactCreated(data, isReplay) {
    addArtifactRecord(data, true);

    const card = document.createElement('div');
    card.className = 'artifact-stream-card';
    card.innerHTML = `
        <div class="artifact-card-left">
            <div class="artifact-kind-icon">📑</div>
            <div class="artifact-card-meta">
                <span class="artifact-card-title">${escapeHtml(data.title || 'Artifact')}</span>
                <span class="artifact-card-path">${escapeHtml(data.path || '')}</span>
            </div>
        </div>
        <button type="button" class="artifact-open-btn">Open in Inspector →</button>
    `;
    card.addEventListener('click', () => inspectArtifact(data));
    streamContainer.appendChild(card);

    if (!isReplay) openInspector('workspace', true);
}

// Permission Card
function renderPermissionPrompt(permId, tool, description, args) {
    const existing = document.getElementById(`perm-${permId}`);
    if (existing) return existing;

    const card = document.createElement('div');
    card.id = `perm-${permId}`;
    card.className = 'permission-card';
    // The agent is BLOCKED until a human answers this. It must be impossible to
    // miss: announced to screen readers, and scrolled into view even when the
    // reader has auto-scroll turned off to read earlier output.
    card.setAttribute('role', 'alert');
    card.setAttribute('aria-live', 'assertive');
    // Explicit "still waiting" marker: the badge's click handler only jumps to
    // cards that still carry it, and it is removed the moment it is answered.
    card.setAttribute('data-pending', '1');
    card.innerHTML = `
        <div class="permission-card-head">
            <span class="perm-shield-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg></span>
            <span>Waiting for your approval — the agent is paused (${escapeHtml(tool || 'tool')})</span>
        </div>
        <div class="permission-card-desc">${escapeHtml(description)}</div>
        <div class="permission-card-actions">
            <button class="perm-btn deny" data-perm-id="${permId}" data-perm-approved="0">Deny</button>
            <button class="perm-btn approve" data-perm-id="${permId}" data-perm-approved="1">Approve</button>
            <button class="perm-btn always" data-perm-id="${permId}" data-perm-approved="1" data-perm-always="1">Approve & always allow</button>
        </div>
    `;
    streamContainer.appendChild(card);
    // Force it on screen regardless of the reader's scroll position.
    try {
        card.scrollIntoView({ behavior: 'smooth', block: 'center' });
    } catch (_) {
        card.scrollIntoView();
    }
    markPendingPermission(permId, tool);
    return card;
}

/// Track how many approvals are outstanding, and reflect them in the document
/// title / top bar so a stalled run is never mistaken for a working one.
const __pendingPerms = new Set();
function markPendingPermission(permId, tool) {
    __pendingPerms.add(permId);
    syncPermissionBadge();
}
function clearPendingPermission(permId) {
    __pendingPerms.delete(permId);
    syncPermissionBadge();
}
/// Forget every outstanding approval, e.g. when the stream is torn down.
/// Without this the badge keeps claiming "1 awaiting approval" for cards that
/// no longer exist (session switch, opening history, new chat).
function clearAllPendingPermissions() {
    __pendingPerms.clear();
    syncPermissionBadge();
}
function syncPermissionBadge() {
    const n = __pendingPerms.size;
    document.body.classList.toggle('has-pending-permission', n > 0);
    const badge = document.getElementById('permission-badge');
    if (badge) {
        badge.textContent = n > 0 ? `${n} awaiting approval` : '';
        badge.classList.toggle('hidden', n === 0);
        badge.onclick = () => {
            // Only the cards that are still waiting: an answered card keeps no
            // alert role, so the badge can never jump to a dead prompt.
            const first = document.querySelector('.permission-card[data-pending="1"]');
            if (first) {
                first.scrollIntoView({ behavior: 'smooth', block: 'center' });
                const approve = first.querySelector('.perm-btn.approve');
                if (approve) approve.focus();
            }
        };
    }
}
function pendingPermissionCount() {
    return __pendingPerms.size;
}

window.decidePermission = async function(permId, approved, alwaysAllow = false) {
    try {
        const res = await fetch(`/api/permissions/${permId}/decision`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ approved, always_allow: !!alwaysAllow })
        });
        if (res.ok) {
            updatePermissionPromptDecision(permId, approved, alwaysAllow);
            clearPendingPermission(permId);
        } else {
            showToast('Could not record that decision (HTTP ' + res.status + ').', 'error', 7000);
        }
    } catch (e) {
        showToast('Permission decision error: ' + e.message, 'error', 7000);
    }
};

function updatePermissionPromptDecision(permId, approved, alwaysAllow = false, reason = null) {
    clearPendingPermission(permId);
    const card = document.getElementById(`perm-${permId}`);
    if (card) {
        const actions = card.querySelector('.permission-card-actions');
        if (actions) {
            // A timeout or a Stop is NOT a denial. Label it the way it happened.
            const label = approved
                ? (alwaysAllow ? 'Approved - always allowed' : 'Approved')
                : (reason === 'timeout'
                    ? 'Not answered - timed out after 120s'
                    : (reason === 'stopped' ? 'Cancelled - the turn was stopped' : 'Denied'));
            actions.innerHTML = `
                <span style="font-size: 0.8rem; font-weight: 600; color: ${approved ? 'var(--accent-green)' : 'var(--accent-red)'};">
                    ${approved ? '✓' : '✕'} ${label}
                </span>
            `;
        }
        card.removeAttribute('role');
        card.removeAttribute('data-pending');
        card.setAttribute('aria-live', 'off');
        card.setAttribute('data-decided', '1');
    }
}

function finishSession(finalAnswer, steps, durationSecs, isReplay = false) {
    streamContainer.querySelectorAll('.reasoning-accordion .pulse-dot').forEach(dot => {
        dot.classList.add('idle');
    });

    if (activeReasoningDrawer && activeDisplayVerbosity !== 'verbose' && activeReasoningDrawer.dataset.userToggled !== '1') {
        activeReasoningDrawer.open = false;
    }

    const footerBar = document.createElement('div');
    footerBar.className = 'msg-footer-bar';
    const secs = typeof durationSecs === 'number' ? durationSecs.toFixed(1) : '0.0';
    footerBar.innerHTML = isReplay
        ? `<span>Task completed (${steps} turn${steps === 1 ? '' : 's'} · ${secs}s)</span>`
        : `<span>Task completed (${steps} turn${steps === 1 ? '' : 's'} · ${secs}s)</span><span>·</span><span>just now</span>`;
    streamContainer.appendChild(footerBar);

    if (!isReplay) {
        btnSubmit.disabled = false;
        if (swarmCoordinator) swarmCoordinator.status = 'completed';
        setSessionRunning(false);
        scheduleSwarmStateSync();
        fetchSessions();
    }
}

function appendSystemError(msg) {
    const el = document.createElement('div');
    el.className = 'code-block-card';
    el.style.borderColor = 'var(--accent-red)';
    el.innerHTML = `
        <div class="code-block-header" style="color: var(--accent-red);"><span>Error</span></div>
        <div class="code-block-body" style="color: var(--accent-red);">${escapeHtml(msg)}</div>
    `;
    streamContainer.appendChild(el);
}

function appendSystemWarning(msg) {
    const el = document.createElement('div');
    el.className = 'code-block-card';
    el.style.borderColor = 'var(--accent-amber)';
    el.innerHTML = `
        <div class="code-block-header" style="color: var(--accent-amber);"><span>Notice</span></div>
        <div class="code-block-body" style="color: #fef08a;">${escapeHtml(msg)}</div>
    `;
    streamContainer.appendChild(el);
}

// Historical Session Viewer
async function loadHistoricalSession(id, task) {
    if (sseReconnectTimer) {
        clearTimeout(sseReconnectTimer);
        sseReconnectTimer = null;
    }
    sseReconnectAttempts = 0;
    if (activeEventSource) {
        activeEventSource.close();
        activeEventSource = null;
    }
    activeEventStreamSessionId = null;
    currentSessionId = id;
    activeTurnId = null;
    queuedPrompts = [];
    renderQueuedPromptsTray();
    setSessionRunning(false);

    updateBreadcrumbSessionUI(task || 'Conversation');
    if (welcomeCanvas) welcomeCanvas.style.display = 'none';
    streamContainer.innerHTML = '';
    // Any outstanding approval belonged to the stream we just discarded.
    clearAllPendingPermissions();
    activeReasoningDrawer = null;
    activeMessageBox = null;
    resetInspectorState();
    renderProjectsTree();

    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(id)}/history`);
        if (currentSessionId !== id) return;
        if (res.ok) {
            const events = await res.json();
            if (currentSessionId !== id) return;
            replayHistoryEvents(Array.isArray(events) ? events : []);
        }
        await Promise.all([
            fetchSessionQueue(id),
            fetchSessionSwarmState(id),
            fetchSessions()
        ]);
        if (currentSessionId !== id) return;

        const sess = allSessions.find(s => s.id === id);
        if (sess) {
            if (sess.knowledge_only || sess.project_id === CONVERSATIONS_PROJECT_ID) {
                activeComposerScope = 'conversation';
            } else if (sess.project_id) {
                activeComposerScope = 'project';
                activeProjectId = sess.project_id;
            }
            await applyConversationConfig(sess);
            renderProjectsTree();
        }
        if (sess && sess.status === 'running') {
            setSessionRunning(true);
            connectEventStream(id);
        } else {
            finalizeOrphanedStreamStates();
            setSessionRunning(false);
        }
        scrollToBottom(true);
    } catch (_) {}
}

// ==========================================================================
// Enhanced Model Selector, Reasoning Capabilities & Provider Management
// ==========================================================================
function getExperimentalModelsList() {
    return [
        {
            id: "Google-Search-Model",
            owned_by: "google-search-ai",
            experimental: true,
            max_context_tokens: 32768,
            reasoning: { support: "none", levels: [], default_level: null, source: "experimental_builtin" },
            modalities: { text: true, vision: false, video: false, audio: false, files: false, pdf: false, tools: true, source: "experimental_builtin" }
        }
    ];
}

function seedExperimentalModelCaches() {
    getExperimentalModelsList().forEach(m => {
        if (!m || !m.id) return;
        if (m.reasoning) {
            modelReasoningCache.set(m.id, m.reasoning);
        }
        if (m.modalities) {
            modelModalitiesCache.set(m.id, m.modalities);
        }
        if (typeof m.max_context_tokens === 'number' && m.max_context_tokens > 0) {
            modelContextCache.set(m.id, m.max_context_tokens);
        }
    });
}

function mergeWithExperimentalModels(models) {
    const list = Array.isArray(models) ? models : [];
    const expList = getExperimentalModelsList();
    const expById = new Map(expList.map(m => [m.id.toLowerCase(), m]));
    const seen = new Set();
    const merged = [];

    list.forEach(m => {
        if (!m || !m.id) return;
        const key = m.id.toLowerCase();
        if (seen.has(key)) return;
        seen.add(key);
        const expOverride = expById.get(key);
        if (expOverride) {
            merged.push({
                ...expOverride,
                ...m,
                experimental: true,
                max_context_tokens: (typeof m.max_context_tokens === 'number' && m.max_context_tokens > 0)
                    ? m.max_context_tokens
                    : expOverride.max_context_tokens,
                reasoning: (m.reasoning && m.reasoning.support)
                    ? m.reasoning
                    : expOverride.reasoning
            });
        } else {
            merged.push(m);
        }
    });

    expList.forEach(exp => {
        const key = exp.id.toLowerCase();
        if (!seen.has(key)) {
            seen.add(key);
            merged.push(exp);
        }
    });

    seedExperimentalModelCaches();
    return merged;
}

function getFallbackModelsList() {
    // No providers are configured by default, so there is no fallback catalogue.
    // The model picker is populated from real provider model lists only.
    return [];
}
function getModelReasoningSpec(modelId) {
    const id = (modelId || '').trim();
    if (!id) return { support: 'none', levels: [], default_level: null, source: 'empty' };
    if (modelReasoningCache.has(id)) return modelReasoningCache.get(id);

    const remoteMatch = allRemoteModels.find(m => m.id === id);
    if (remoteMatch && remoteMatch.reasoning) {
        modelReasoningCache.set(id, remoteMatch.reasoning);
        return remoteMatch.reasoning;
    }
    const fallbackMatch = getFallbackModelsList().find(m => m.id === id);
    if (fallbackMatch && fallbackMatch.reasoning) {
        return fallbackMatch.reasoning;
    }

    // Immediate client fallback while backend /api/models/reasoning resolves
    const low = id.toLowerCase();
    if (low.includes('google-search-model')) {
        return { support: 'none', levels: [], default_level: null, source: 'experimental_builtin' };
    }
    if (low.includes('deepseek-r1') || low.includes('qwq') || low.includes('qvq')) {
        return { support: 'fixed', levels: [], default_level: null, source: 'heuristic' };
    }
    if (low.includes('nemotron') || low.includes('o1') || low.includes('o3') || low.includes('o4') || low.includes('gpt-5') || low.includes('gpt-oss') || low.includes('claude') || low.includes('gemini-2.5') || low.includes('gemini-3') || low.includes('thinking') || low.includes('reasoning')) {
        return { support: 'configurable', levels: ['low', 'medium', 'high'], default_level: 'high', source: 'heuristic' };
    }
    return { support: 'none', levels: [], default_level: null, source: 'heuristic' };
}

function inferFallbackModelModalities(modelId) {
    const id = String(modelId || '').toLowerCase();
    if (!id) {
        return { text: true, vision: false, video: false, audio: false, files: false, pdf: false, tools: true, source: 'fallback' };
    }
    if (id.includes('google-search-model')) {
        return { text: true, vision: false, video: false, audio: false, files: false, pdf: false, tools: true, source: 'experimental_builtin' };
    }
    if (id.includes('gemini')) {
        return { text: true, vision: true, video: true, audio: true, files: true, pdf: true, tools: true, source: 'family' };
    }
    if (id.includes('claude')) {
        return { text: true, vision: true, video: false, audio: false, files: true, pdf: true, tools: true, source: 'family' };
    }
    if (id.includes('gpt-4o') || id.includes('gpt-4.1') || id.includes('gpt-4.5') || id.includes('gpt-5') || id.includes('o1') || id.includes('o3') || id.includes('o4')) {
        return { text: true, vision: true, video: false, audio: id.includes('audio') || id.includes('gpt-4o') || id.includes('gpt-5'), files: true, pdf: true, tools: true, source: 'family' };
    }
    if (id.includes('nemotron') && (id.includes('vl') || id.includes('omni') || id.includes('vision'))) {
        return { text: true, vision: true, video: id.includes('omni') || id.includes('video'), audio: id.includes('omni') || id.includes('audio'), files: true, pdf: false, tools: true, source: 'family' };
    }
    if (id.includes('qwen') && (id.includes('vl') || id.includes('omni'))) {
        return { text: true, vision: true, video: id.includes('vl') || id.includes('omni'), audio: id.includes('omni'), files: false, pdf: false, tools: true, source: 'family' };
    }
    if (id.includes('pixtral') || id.includes('mistral-large-4') || id.includes('llama-3.2') && id.includes('vision') || id.includes('llama-4') || id.includes('gemma-3') || id.includes('qvq') || id.includes('-vl') || id.includes('vision')) {
        return { text: true, vision: true, video: false, audio: false, files: false, pdf: false, tools: true, source: 'family' };
    }
    return { text: true, vision: false, video: false, audio: false, files: false, pdf: false, tools: true, source: 'family' };
}

function getModelModalityCaps(modelId) {
    const id = (modelId || '').trim();
    if (!id) return inferFallbackModelModalities('');
    if (modelModalitiesCache.has(id)) return modelModalitiesCache.get(id);
    const remoteMatch = allRemoteModels.find(m => m.id === id);
    if (remoteMatch && remoteMatch.modalities) {
        modelModalitiesCache.set(id, remoteMatch.modalities);
        return remoteMatch.modalities;
    }
    return inferFallbackModelModalities(id);
}

async function checkModelCapabilities(modelId) {
    const id = (modelId || '').trim();
    if (!id) return null;
    try {
        const res = await fetch(`/api/models/capabilities?model=${encodeURIComponent(id)}`);
        if (!res.ok) return null;
        const caps = await res.json();
        if (caps && caps.reasoning) modelReasoningCache.set(id, caps.reasoning);
        if (caps && caps.modalities) modelModalitiesCache.set(id, caps.modalities);
        if (caps && typeof caps.max_context_tokens === 'number' && caps.max_context_tokens > 0) {
            modelContextCache.set(id, caps.max_context_tokens);
        }
        renderModelActiveCapsBar();
        renderComposerAttachments();
        return caps;
    } catch (_) {
        return null;
    }
}

function renderModelActiveCapsBar() {
    const activeId = (currentConfig && currentConfig.model) || '';
    if (!activeId || !labelModelCtxSummary) return;
    const ctx = getCachedOrInferredModelContextWindow(activeId);
    labelModelCtxSummary.textContent = `${formatTokenCount(ctx)} ctx`;
}

function inferFallbackModelContextWindow(modelId) {
    const id = String(modelId || '').toLowerCase();
    if (!id) return 128000;
    if (id.includes('google-search-model')) return 32768;
    if (id.includes('gemini-2') || id.includes('gemini-1.5') || id.includes('gemini-3')) return 1000000;
    if (id.includes('claude')) return 200000;
    if (id.includes('o1') || id.includes('o3') || id.includes('o4') || id.includes('gpt-5')) return 200000;
    if (id.includes('qwen')) return 131072;
    if (id.includes('llama-3.1') || id.includes('llama-3.2') || id.includes('llama-3.3') || id.includes('llama-4')) return 128000;
    if (id.includes('nemotron') || id.includes('deepseek') || id.includes('gpt-4') || id.includes('gpt-oss')) return 128000;
    if (id.includes('gemma-3-12b') || id.includes('gemma-2')) return 32768;
    return 128000;
}

function getCachedOrInferredModelContextWindow(modelId) {
    const id = (modelId || '').trim();
    if (!id) return 128000;
    if (modelContextCache.has(id)) return modelContextCache.get(id);
    const remoteMatch = allRemoteModels.find(m => m.id === id);
    if (remoteMatch && typeof remoteMatch.max_context_tokens === 'number' && remoteMatch.max_context_tokens > 0) {
        modelContextCache.set(id, remoteMatch.max_context_tokens);
        return remoteMatch.max_context_tokens;
    }
    return inferFallbackModelContextWindow(id);
}

async function resolveModelContextWindow(modelId) {
    const id = (modelId || '').trim();
    if (!id) return 128000;
    if (modelContextCache.has(id)) return modelContextCache.get(id);
    const remoteMatch = allRemoteModels.find(m => m.id === id);
    if (remoteMatch && typeof remoteMatch.max_context_tokens === 'number' && remoteMatch.max_context_tokens > 0) {
        modelContextCache.set(id, remoteMatch.max_context_tokens);
        return remoteMatch.max_context_tokens;
    }
    try {
        const res = await fetch(`/api/models/reasoning?model=${encodeURIComponent(id)}`);
        if (res.ok) {
            const spec = await res.json();
            if (spec && spec.support) modelReasoningCache.set(id, spec);
            if (spec && spec.modalities) modelModalitiesCache.set(id, spec.modalities);
            if (spec && typeof spec.max_context_tokens === 'number' && spec.max_context_tokens > 0) {
                modelContextCache.set(id, spec.max_context_tokens);
                return spec.max_context_tokens;
            }
        }
    } catch (_) {}
    return inferFallbackModelContextWindow(id);
}

function isAutoDetectContextEnabled() {
    if (cfgAutoDetectContext) return !!cfgAutoDetectContext.checked;
    return currentConfig?.memory?.auto_detect_context !== false;
}

function syncAutoContextUiState() {
    const autoOn = isAutoDetectContextEnabled();
    if (cfgMaxContextTokens) cfgMaxContextTokens.disabled = autoOn;
    if (cfgMaxContextAutoHint) cfgMaxContextAutoHint.classList.toggle('hidden', !autoOn);
}

function applyResolvedContextWindow(maxCtx) {
    const validMax = (typeof maxCtx === 'number' && maxCtx > 0) ? maxCtx : 128000;
    if (cfgMaxContextTokens && isAutoDetectContextEnabled()) {
        cfgMaxContextTokens.value = validMax;
    }
    if (currentConfig) {
        if (!currentConfig.memory) currentConfig.memory = {};
        if (isAutoDetectContextEnabled()) {
            currentConfig.memory.max_context_tokens = validMax;
        }
    }
    const effectiveMax = currentConfig?.memory?.max_context_tokens || validMax;
    const current = lastCurrentContextTokens || 0;
    const pct = effectiveMax > 0 ? Math.min(100, Math.round((current / effectiveMax) * 1000) / 10) : 0;

    if (statCtxTokens) statCtxTokens.textContent = `${formatTokenCount(current)} / ${formatTokenCount(effectiveMax)}`;
    if (statCtxPct) statCtxPct.textContent = `${pct}%`;
    if (labelModelCtxSummary) labelModelCtxSummary.textContent = `${formatTokenCount(effectiveMax)} ctx`;
    if (analyticsCtxUtilText) analyticsCtxUtilText.textContent = `${current.toLocaleString()} / ${formatTokenCount(effectiveMax)} (${pct}%)`;
    if (analyticsModelWindowHint) {
        analyticsModelWindowHint.textContent = `Model Window: ${formatTokenCount(effectiveMax)} tokens · Automatic prompt caching active`;
    }
}

async function syncModelContextWindowIfAuto(modelId) {
    syncAutoContextUiState();
    if (!isAutoDetectContextEnabled()) return;
    const immediate = getCachedOrInferredModelContextWindow(modelId);
    applyResolvedContextWindow(immediate);
    const resolved = await resolveModelContextWindow(modelId);
    applyResolvedContextWindow(resolved);
}

async function resolveModelReasoningSpec(modelId) {
    const id = (modelId || '').trim();
    if (!id) return getModelReasoningSpec(id);
    if (modelReasoningCache.has(id)) return modelReasoningCache.get(id);
    try {
        const res = await fetch(`/api/models/reasoning?model=${encodeURIComponent(id)}`);
        if (res.ok) {
            const spec = await res.json();
            if (spec && typeof spec.max_context_tokens === 'number' && spec.max_context_tokens > 0) {
                modelContextCache.set(id, spec.max_context_tokens);
            }
            if (spec && spec.modalities) {
                modelModalitiesCache.set(id, spec.modalities);
            }
            if (spec && spec.support) {
                modelReasoningCache.set(id, spec);
                return spec;
            }
        }
    } catch (_) {}
    return getModelReasoningSpec(id);
}

function renderReasoningUiForModel(modelId, requestedEffort) {
    const spec = getModelReasoningSpec(modelId);
    const badgeMeta = getDefaultReasoningBadgeMeta(spec);

    let effectiveEffort = requestedEffort || spec.default_level || 'high';

    if (spec.support === 'configurable' && Array.isArray(spec.levels) && spec.levels.length > 0) {
        if (!spec.levels.includes(effectiveEffort)) {
            effectiveEffort = spec.default_level && spec.levels.includes(spec.default_level)
                ? spec.default_level
                : spec.levels[spec.levels.length - 1];
        }

        if (labelReasoningEffort) {
            labelReasoningEffort.textContent = formatReasoningLevelLabel(effectiveEffort);
            labelReasoningEffort.classList.remove('hidden');
        }
        if (btnModelDropdown) {
            btnModelDropdown.title = `Model & thinking effort (${spec.levels.map(formatReasoningLevelLabel).join(', ')})`;
        }
        if (cfgReasoningCapabilityBadge) {
            cfgReasoningCapabilityBadge.className = 'reasoning-mode-badge configurable';
            cfgReasoningCapabilityBadge.textContent = badgeMeta.text;
        }

        setCustomSelectOptions(
            'cfg-reasoning-select',
            spec.levels.map(lvl => ({ value: lvl, label: formatReasoningLevelLabel(lvl) })),
            effectiveEffort,
            null
        );
    } else if (spec.support === 'fixed') {
        if (labelReasoningEffort) {
            labelReasoningEffort.textContent = '';
            labelReasoningEffort.classList.add('hidden');
        }
        if (btnModelDropdown) {
            btnModelDropdown.title = 'Select model (fixed always-on reasoning)';
        }
        if (cfgReasoningCapabilityBadge) {
            cfgReasoningCapabilityBadge.className = 'reasoning-mode-badge fixed';
            cfgReasoningCapabilityBadge.textContent = 'Fixed (Always On)';
        }
        setCustomSelectOptions('cfg-reasoning-select', [], effectiveEffort, 'Fixed (always on)');
    } else {
        if (labelReasoningEffort) {
            labelReasoningEffort.textContent = '';
            labelReasoningEffort.classList.add('hidden');
        }
        if (btnModelDropdown) {
            btnModelDropdown.title = 'Select model';
        }
        if (cfgReasoningCapabilityBadge) {
            cfgReasoningCapabilityBadge.className = 'reasoning-mode-badge unsupported';
            cfgReasoningCapabilityBadge.textContent = 'Not Exposed';
        }
        setCustomSelectOptions('cfg-reasoning-select', [], effectiveEffort, 'Not exposed by model');
    }

    return effectiveEffort;
}

async function refreshReasoningControlsForModel(modelId, persistIfAdjusted = false) {
    const spec = await resolveModelReasoningSpec(modelId);
    const currentEffort = (currentConfig && currentConfig.reasoning_effort)
        || (cfgReasoningEffort && cfgReasoningEffort.value)
        || 'high';
    const resolvedEffort = renderReasoningUiForModel(modelId, currentEffort);

    if (currentConfig && spec.support === 'configurable' && currentEffort !== resolvedEffort) {
        currentConfig.reasoning_effort = resolvedEffort;
        if (persistIfAdjusted) {
            persistCurrentConfigInBackground();
        }
    }
}

function updateModelPillDisplay() {
    if (!currentConfig) return;
    const orch = currentConfig.orchestrator || {};
    const orchOn = Boolean(orch.enabled);
    const activeModel = orchOn && orch.model ? orch.model : (currentConfig.model || 'Nemotron-3-Nano');
    const shortName = activeModel.split('/').pop();
    if (labelActiveModel) {
        labelActiveModel.textContent = orchOn ? `Orchestrator · ${shortName}` : shortName;
    }
    const effort = orchOn ? (orch.thinking_effort || currentConfig.reasoning_effort || 'high') : (currentConfig.reasoning_effort || 'high');
    renderReasoningUiForModel(activeModel, effort);
    const maxCtx = getCachedOrInferredModelContextWindow(activeModel);
    if (labelModelCtxSummary) {
        labelModelCtxSummary.textContent = `${formatTokenCount(maxCtx)} ctx`;
    }
    if (btnModelDropdown) {
        const provId = orchOn ? (orch.provider_id || currentConfig.provider) : currentConfig.provider;
        const provObj = (currentConfig.custom_providers || []).find(p => p.id === provId);
        const provLabel = provObj ? provObj.name : (provId || 'Default');
        btnModelDropdown.title = orchOn
            ? `Orchestrator: ${activeModel} (${effort}) · ${formatTokenCount(maxCtx)} ctx\nProvider: ${provLabel}\nSubagent: ${(orch.subagent_model || 'inherit')} on ${(orch.subagent_server_id || 'local')}`
            : `Model: ${activeModel} (${effort}) · ${formatTokenCount(maxCtx)} ctx\nProvider: ${provLabel}`;
    }
}

function showModelHoverDetail(html) {
    if (!modelHoverDetail) return;
    modelHoverDetail.innerHTML = html;
}

function resetModelHoverDetail() {
    if (!modelHoverDetail) return;
    modelHoverDetail.innerHTML = '<div class="model-hover-detail-meta">Hover a model to view context length &amp; capabilities</div>';
}

function positionModelPopover(anchorEl) {
    if (!modelPopover || !anchorEl) return;
    modelPopover.classList.remove('hidden');
    const rect = anchorEl.getBoundingClientRect();
    const popWidth = modelPopover.offsetWidth || 348;
    const popHeight = modelPopover.offsetHeight || 430;
    const maxLeft = Math.max(8, window.innerWidth - popWidth - 8);
    modelPopover.style.left = `${Math.max(8, Math.min(rect.left, maxLeft))}px`;

    const spaceAbove = rect.top - 12;
    const spaceBelow = window.innerHeight - rect.bottom - 12;
    if (spaceAbove >= popHeight || spaceAbove >= spaceBelow) {
        modelPopover.style.top = '';
        modelPopover.style.bottom = `${Math.max(8, window.innerHeight - rect.top + 8)}px`;
    } else {
        modelPopover.style.bottom = '';
        modelPopover.style.top = `${Math.max(8, Math.min(rect.bottom + 8, window.innerHeight - popHeight - 8))}px`;
    }
}

function openUnifiedModelPicker(anchorEl, pickerCtx = null) {
    if (!modelPopover || !anchorEl) return;
    activeModelPickerContext = pickerCtx || null;
    const titleEl = modelPopover.querySelector('.model-popover-title');
    if (titleEl) {
        titleEl.textContent = (pickerCtx && pickerCtx.title) ? pickerCtx.title : 'Model';
    }
    if (modelSearchInput) {
        modelSearchInput.value = '';
        if (btnClearModelSearch) btnClearModelSearch.classList.add('hidden');
    }
    resetModelHoverDetail();
    hideThinkingEffortSubmenu();
    renderFilteredModels(false);
    positionModelPopover(anchorEl);
    if (quickModelsList) quickModelsList.scrollTop = 0;
    if (modelSearchInput) modelSearchInput.focus({ preventScroll: true });
    if (allRemoteModels.length === 0) fetchRemoteModels();
    if (!pickerCtx && currentConfig && currentConfig.model) {
        checkModelCapabilities(currentConfig.model);
    }
}

function getAllModelsWithProviders() {
    if (allProviderModelGroups.length > 0) {
        return allProviderModelGroups;
    }
    // No provider configured: return nothing rather than inventing a provider.
    const providers = (currentConfig && currentConfig.custom_providers) || [];
    const activeProv = providers.find(p => p.id === (currentConfig && currentConfig.provider));
    if (!activeProv && allRemoteModels.length === 0) {
        return [];
    }
    return [{
        provider_id: (activeProv && activeProv.id) || (currentConfig && currentConfig.provider) || '',
        provider_name: (activeProv && activeProv.name) || '',
        is_active: true,
        has_credentials: !!activeProv,
        models: allRemoteModels
    }];
}

function modelMatchesCapsAndCtx(m) {
    const ctxTokens = (typeof m.max_context_tokens === 'number' && m.max_context_tokens > 0)
        ? m.max_context_tokens
        : getCachedOrInferredModelContextWindow(m.id);
    if (activeCtxMinFilter > 0 && ctxTokens < activeCtxMinFilter) {
        return false;
    }
    if (activeCapFilters.size > 0) {
        const mods = m.modalities || getModelModalityCaps(m.id) || {};
        const spec = m.reasoning || getModelReasoningSpec(m.id);
        for (const cap of activeCapFilters) {
            if (cap === 'reasoning') {
                if (spec.support === 'none') return false;
            } else if (!mods[cap]) {
                return false;
            }
        }
    }
    return true;
}

async function selectModel(modelId, effortOverride = null, providerIdOverride = null) {
    const spec = await resolveModelReasoningSpec(modelId);
    const pickerCtx = activeModelPickerContext;
    let nextEffort = effortOverride || (pickerCtx ? pickerCtx.selectedEffort : (currentConfig && currentConfig.reasoning_effort)) || 'high';
    if (spec.support === 'configurable' && Array.isArray(spec.levels) && spec.levels.length > 0 && !spec.levels.includes(nextEffort)) {
        nextEffort = (spec.default_level && spec.levels.includes(spec.default_level))
            ? spec.default_level
            : spec.levels[spec.levels.length - 1];
    }

    if (pickerCtx && typeof pickerCtx.onSelect === 'function') {
        const cb = pickerCtx.onSelect;
        activeModelPickerContext = null;
        if (modelPopover) modelPopover.classList.add('hidden');
        hideThinkingEffortSubmenu();
        cb({
            model: modelId,
            provider_id: providerIdOverride || pickerCtx.selectedProviderId || '',
            thinking_effort: nextEffort
        });
        return;
    }

    if (labelActiveModel) labelActiveModel.textContent = modelId.split('/').pop();
    if (cfgModel) cfgModel.value = modelId;

    if (currentConfig) {
        if (providerIdOverride && Array.isArray(currentConfig.custom_providers)) {
            const targetProv = currentConfig.custom_providers.find(p => p.id === providerIdOverride);
            if (targetProv) {
                currentConfig.custom_providers.forEach(p => { p.is_active = (p.id === targetProv.id); });
                currentConfig.provider = targetProv.id;
                if (targetProv.api_base) currentConfig.api_base = targetProv.api_base;
                if (targetProv.api_key) currentConfig.api_key = targetProv.api_key;
                if (cfgApiBase) cfgApiBase.value = currentConfig.api_base || '';
                if (cfgApiKey) cfgApiKey.value = currentConfig.api_key || '';
                if (labelActiveProviderName) labelActiveProviderName.textContent = targetProv.name;
            }
        }
        currentConfig.model = modelId;
        currentConfig.reasoning_effort = nextEffort;
        if (isAutoDetectContextEnabled()) {
            const nextCtx = await resolveModelContextWindow(modelId);
            if (!currentConfig.memory) currentConfig.memory = {};
            currentConfig.memory.max_context_tokens = nextCtx;
            applyResolvedContextWindow(nextCtx);
        }
        persistCurrentConfigInBackground();
    } else {
        await syncModelContextWindowIfAuto(modelId);
    }
    updateModelPillDisplay();
    renderModelActiveCapsBar();
    renderComposerAttachments();
    checkModelCapabilities(modelId);
    renderFilteredModels(true);
    renderConfiguredProviders();
    syncActiveConversationConfig();
    closeAllPopovers();
}

function scheduleCloseThinkingSubmenu() {
    if (submenuCloseTimer) clearTimeout(submenuCloseTimer);
    submenuCloseTimer = setTimeout(() => {
        hideThinkingEffortSubmenu();
    }, 120);
}

function hideThinkingEffortSubmenu() {
    if (submenuCloseTimer) {
        clearTimeout(submenuCloseTimer);
        submenuCloseTimer = null;
    }
    submenuHoverModelId = null;
    if (thinkingEffortSubmenu) thinkingEffortSubmenu.classList.add('hidden');
    if (quickModelsList) {
        quickModelsList.querySelectorAll('.model-entry-card.submenu-open').forEach(el => el.classList.remove('submenu-open'));
    }
}

function showThinkingEffortSubmenu(rowEl, modelObj, spec, providerId = null) {
    if (!thinkingEffortSubmenu || !thinkingEffortList || !rowEl || !modelPopover) return;
    if (!spec || spec.support !== 'configurable' || !Array.isArray(spec.levels) || spec.levels.length === 0) {
        hideThinkingEffortSubmenu();
        return;
    }
    if (submenuCloseTimer) {
        clearTimeout(submenuCloseTimer);
        submenuCloseTimer = null;
    }
    submenuHoverModelId = modelObj.id;

    if (quickModelsList) {
        quickModelsList.querySelectorAll('.model-entry-card.submenu-open').forEach(el => {
            el.classList.toggle('submenu-open', el === rowEl);
        });
    }

    const ctxSelModel = activeModelPickerContext ? (activeModelPickerContext.selectedModel || '') : ((currentConfig && currentConfig.model) || '');
    const ctxSelProv = activeModelPickerContext ? (activeModelPickerContext.selectedProviderId || '') : ((currentConfig && currentConfig.provider) || '');
    const ctxSelEffort = activeModelPickerContext ? (activeModelPickerContext.selectedEffort || '') : ((currentConfig && currentConfig.reasoning_effort) || '');
    const isActiveModel = modelObj.id === ctxSelModel && (!providerId || !ctxSelProv || ctxSelProv === providerId);
    const currentEffort = isActiveModel
        ? (ctxSelEffort || spec.default_level || 'high')
        : (spec.default_level && spec.levels.includes(spec.default_level) ? spec.default_level : spec.levels[spec.levels.length - 1]);

    if (thinkingSubmenuModelHint) {
        thinkingSubmenuModelHint.title = `${modelObj.id.split('/').pop()} · ${spec.levels.map(formatReasoningLevelLabel).join(' / ')}`;
    }

    thinkingEffortList.innerHTML = '';
    spec.levels.forEach(lvl => {
        const isChosen = isActiveModel ? (lvl === currentEffort) : false;
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `popover-item${isChosen ? ' active' : ''}`;
        btn.setAttribute('data-effort', lvl);
        btn.innerHTML = `<span class="popover-item-title">${escapeHtml(formatReasoningLevelLabel(lvl))}</span>${isChosen ? '<span class="popover-item-check">✓</span>' : ''}`;
        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            selectModel(modelObj.id, lvl, providerId || modelObj.provider_id || null);
        });
        thinkingEffortList.appendChild(btn);
    });

    thinkingEffortSubmenu.classList.remove('hidden');
    const popRect = modelPopover.getBoundingClientRect();
    const rowRect = rowEl.getBoundingClientRect();
    const subWidth = thinkingEffortSubmenu.offsetWidth || 185;
    const subHeight = thinkingEffortSubmenu.offsetHeight || 180;

    let left = popRect.right + 6;
    if (left + subWidth > window.innerWidth - 8) {
        left = Math.max(8, popRect.left - subWidth - 6);
    }
    let top = rowRect.top - 6;
    if (top + subHeight > window.innerHeight - 8) {
        top = Math.max(8, window.innerHeight - subHeight - 8);
    }
    top = Math.max(8, top);

    thinkingEffortSubmenu.style.left = `${left}px`;
    thinkingEffortSubmenu.style.top = `${top}px`;
    thinkingEffortSubmenu.style.bottom = '';
}

function renderFilteredModels(preserveScroll = true) {
    if (!quickModelsList) return;
    const prevScroll = preserveScroll ? quickModelsList.scrollTop : 0;
    const query = modelSearchInput ? modelSearchInput.value : '';
    const activeModelId = activeModelPickerContext ? (activeModelPickerContext.selectedModel || '') : ((currentConfig && currentConfig.model) || '');
    const activeProviderId = activeModelPickerContext ? (activeModelPickerContext.selectedProviderId || '') : ((currentConfig && currentConfig.provider) || '');
    const activeEffort = activeModelPickerContext ? (activeModelPickerContext.selectedEffort || 'high') : ((currentConfig && currentConfig.reasoning_effort) || 'high');
    const hideOrch = Boolean(activeModelPickerContext && activeModelPickerContext.hideOrchestrator);
    const orch = (currentConfig && currentConfig.orchestrator) || {};
    const orchOn = Boolean(orch.enabled);

    quickModelsList.innerHTML = '';

    // Dedicated Orchestrator Mode card at top when in main chat picker
    if (!hideOrch) {
        const orchModelName = (orch.model || activeModelId || 'Active Model').split('/').pop();
        const orchCtx = getCachedOrInferredModelContextWindow(orch.model || activeModelId);
        const orchSubName = (orch.subagent_model || 'inherit').split('/').pop();
        const orchRow = document.createElement('button');
        orchRow.type = 'button';
        orchRow.className = `model-entry-card model-orch-row${orchOn ? ' orch-on' : ''}`;
        orchRow.innerHTML = `
            <div class="model-row-main">
                <span class="model-row-name">Orchestrator</span>
                <span class="model-row-effort">${escapeHtml(orchModelName)} · ${escapeHtml(formatReasoningLevelLabel(orch.thinking_effort || 'high'))}</span>
                <span class="model-row-badge orch">${orchOn ? 'ON' : 'OFF'}</span>
            </div>
            <div class="model-row-right">
                <span class="model-row-chevron" data-orch-config="1" title="Configure Orchestrator &amp; Subagents">⚙</span>
            </div>
        `;
        orchRow.addEventListener('mouseenter', () => {
            hideThinkingEffortSubmenu();
            showModelHoverDetail(`
                <div class="model-hover-detail-top">
                    <span>Orchestrator Mode (${orchOn ? 'Enabled' : 'Disabled'})</span>
                    <span class="tag-ctx">${escapeHtml(formatTokenCount(orchCtx))} max ctx</span>
                </div>
                <div class="model-hover-detail-meta">
                    <span>Lead: ${escapeHtml(orchModelName)}</span>
                    <span>·</span>
                    <span>Subagents: ${orch.subagent_mode === 'multi' ? 'Multiple specialized types' : escapeHtml(orchSubName)}</span>
                </div>
            `);
        });
        orchRow.addEventListener('click', (e) => {
            if (e.target.closest('[data-orch-config]')) {
                e.stopPropagation();
                closeAllPopovers();
                openSettingsModal('orchestrator');
                return;
            }
            if (currentConfig) {
                if (!currentConfig.orchestrator) currentConfig.orchestrator = {};
                currentConfig.orchestrator.enabled = !Boolean(currentConfig.orchestrator.enabled);
                persistCurrentConfigInBackground();
                updateModelPillDisplay();
                renderFilteredModels(true);
                syncOrchestratorControls();
                syncActiveConversationConfig();
                showToast(
                    currentConfig.orchestrator.enabled
                        ? 'Orchestrator Mode enabled for new conversations.'
                        : 'Orchestrator Mode disabled.',
                    'info'
                );
            }
        });
        quickModelsList.appendChild(orchRow);
    }

    const groups = getAllModelsWithProviders();
    let totalRendered = 0;

    groups.forEach(group => {
        const filtered = (group.models || []).filter(m => {
            if (!modelMatchesCapsAndCtx(m)) return false;
            if (!query || !query.trim()) return true;
            const shortName = m.id.split('/').pop();
            return matchesModelSearch(query, m.id, shortName, m.owned_by || '', group.provider_name || '');
        });

        if (filtered.length === 0) return;

        const header = document.createElement('div');
        header.className = 'model-provider-header';
        header.innerHTML = `
            <span>${escapeHtml(group.provider_name || group.provider_id)} (${filtered.length})</span>
            <span class="model-provider-badge${group.has_credentials === false ? ' missing-key' : ''}">
                ${group.has_credentials === false ? 'No Key' : 'Ready'}
            </span>
        `;
        quickModelsList.appendChild(header);

        filtered.forEach(m => {
            totalRendered += 1;
            const card = document.createElement('button');
            card.type = 'button';
            const provId = m.provider_id || group.provider_id;
            const isActive = m.id === activeModelId && (!activeProviderId || provId === activeProviderId);
            card.className = `model-entry-card${isActive ? ' active' : ''}`;
            card.title = `${m.id} (${group.provider_name})`;

            const shortName = m.id.split('/').pop();
            const idLower = m.id.toLowerCase();
            const spec = m.reasoning || getModelReasoningSpec(m.id);
            const mods = m.modalities || getModelModalityCaps(m.id) || {};
            const ctxTokens = (typeof m.max_context_tokens === 'number' && m.max_context_tokens > 0)
                ? m.max_context_tokens
                : getCachedOrInferredModelContextWindow(m.id);
            const isExperimental = Boolean(m.experimental) || idLower.includes('google-search-model');
            const hasSubmenu = spec.support === 'configurable' && Array.isArray(spec.levels) && spec.levels.length > 0;

            let effortInlineHtml = '';
            if (hasSubmenu) {
                const rowEffort = isActive
                    ? (spec.levels.includes(activeEffort) ? activeEffort : (spec.default_level || spec.levels[spec.levels.length - 1]))
                    : (spec.default_level && spec.levels.includes(spec.default_level) ? spec.default_level : spec.levels[spec.levels.length - 1]);
                effortInlineHtml = `<span class="model-row-effort">${escapeHtml(formatReasoningLevelLabel(rowEffort))}</span>`;
            } else if (spec.support === 'fixed') {
                effortInlineHtml = `<span class="model-row-effort">Fixed</span>`;
            }

            let badgeHtml = '';
            if (isExperimental) {
                badgeHtml = `<span class="model-row-badge experimental">Experimental</span>`;
            } else {
                badgeHtml = `<span class="model-row-badge">${escapeHtml(formatTokenCount(ctxTokens))}</span>`;
            }

            const rightHtml = hasSubmenu
                ? `<span class="model-row-chevron" aria-hidden="true">›</span>`
                : (isActive ? `<span class="model-row-check">✓</span>` : '');

            card.innerHTML = `
                <div class="model-row-main">
                    <span class="model-row-name">${escapeHtml(shortName)}</span>
                    ${effortInlineHtml}
                    ${badgeHtml}
                </div>
                <div class="model-row-right">${rightHtml}</div>
            `;

            card.addEventListener('mouseenter', () => {
                if (hasSubmenu) {
                    showThinkingEffortSubmenu(card, m, spec, provId);
                } else {
                    hideThinkingEffortSubmenu();
                }
                const capTags = ['vision', 'reasoning', 'tools', 'audio', 'video', 'pdf']
                    .filter(c => c === 'reasoning' ? spec.support !== 'none' : Boolean(mods[c]))
                    .map(c => `<span class="cap-pill supported">${escapeHtml(c)}</span>`)
                    .join('');
                showModelHoverDetail(`
                    <div class="model-hover-detail-top">
                        <span style="overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">${escapeHtml(m.id)}</span>
                        <span class="tag-ctx">${ctxTokens.toLocaleString()} tokens (${escapeHtml(formatTokenCount(ctxTokens))})</span>
                    </div>
                    <div class="model-hover-detail-meta">
                        <span>${escapeHtml(group.provider_name || provId)}</span>
                        <span>·</span>
                        <span>Thinking: ${hasSubmenu ? escapeHtml(spec.levels.join('/')) : escapeHtml(spec.support || 'standard')}</span>
                        ${capTags ? `<span style="margin-left:auto;display:inline-flex;gap:3px;">${capTags}</span>` : ''}
                    </div>
                `);
            });
            card.addEventListener('click', (e) => {
                const clickedChevron = e.target.closest('.model-row-chevron');
                if (clickedChevron && hasSubmenu) {
                    e.stopPropagation();
                    showThinkingEffortSubmenu(card, m, spec, provId);
                    return;
                }
                selectModel(m.id, null, provId);
            });
            quickModelsList.appendChild(card);
        });
    });

    if (totalRendered === 0) {
        const empty = document.createElement('div');
        empty.style.cssText = 'padding: 16px; text-align: center; color: var(--text-muted); font-size: 0.8rem;';
        empty.textContent = 'No models match the search or filters';
        quickModelsList.appendChild(empty);
    }
    if (preserveScroll) {
        quickModelsList.scrollTop = prevScroll;
    }
}

async function fetchRemoteModels() {
    if (btnQuickFetchModels) btnQuickFetchModels.classList.add('spinning');
    if (btnFetchModels) btnFetchModels.textContent = "Fetching...";

    try {
        let groups = [];
        const resAll = await fetch('/api/models/all');
        if (resAll.ok) {
            groups = await resAll.json();
        } else {
            const res = await fetch('/api/models/remote');
            if (!res.ok) throw new Error(`Status ${res.status}`);
            const models = await res.json();
            groups = [{
                provider_id: (currentConfig && currentConfig.provider) || '',
                provider_name: '',
                is_active: true,
                has_credentials: !!(currentConfig && currentConfig.provider),
                models
            }];
        }

        allProviderModelGroups = Array.isArray(groups) ? groups : [];
        const flat = [];
        const seen = new Set();
        allProviderModelGroups.forEach(g => {
            (g.models || []).forEach(m => {
                if (!m || !m.id) return;
                if (m.reasoning) modelReasoningCache.set(m.id, m.reasoning);
                if (m.modalities) modelModalitiesCache.set(m.id, m.modalities);
                if (typeof m.max_context_tokens === 'number' && m.max_context_tokens > 0) {
                    modelContextCache.set(m.id, m.max_context_tokens);
                }
                const key = `${g.provider_id}:${m.id}`;
                if (!seen.has(key)) {
                    seen.add(key);
                    flat.push({ ...m, provider_id: m.provider_id || g.provider_id, provider_name: m.provider_name || g.provider_name });
                }
            });
        });

        allRemoteModels = mergeWithExperimentalModels(flat);

        // Surface per-provider failures: previously these were silently dropped,
        // so a provider returning 401/404 just showed its default model as "Ready".
        const failed = allProviderModelGroups.filter(g => g && g.error);
        if (failed.length > 0) {
            const summary = failed
                .map(g => `${g.provider_name || g.provider_id}: ${g.error}`)
                .join('; ');
            showToast('Provider fetch failed — ' + summary, 'error', 9000);
        }

        renderModelActiveCapsBar();
        renderComposerAttachments();
        renderFilteredModels();
        updateModelPillDisplay();
        if (currentConfig && currentConfig.model) {
            syncModelContextWindowIfAuto(currentConfig.model);
        }
        if (btnQuickFetchModels) {
            btnQuickFetchModels.classList.remove('spinning');
            btnQuickFetchModels.title = `Refreshed ${allRemoteModels.length} models across ${allProviderModelGroups.length} providers`;
        }
        if (btnFetchModels) btnFetchModels.textContent = `✓ ${allRemoteModels.length} Models`;
    } catch (e) {
        seedExperimentalModelCaches();
        renderFilteredModels();
        if (btnQuickFetchModels) btnQuickFetchModels.classList.remove('spinning');
        if (btnFetchModels) btnFetchModels.textContent = "Fetch Failed";
        showToast('Failed to fetch remote models from API: ' + e.message, 'error');
    }
}

// ==========================================================================
// Custom Provider Management
// ==========================================================================
function renderConfiguredProviders() {
    if (!configuredProvidersContainer) return;
    const providers = (currentConfig && currentConfig.custom_providers) || [];
    configuredProvidersContainer.innerHTML = '';

    // No providers yet: show a clear call to action instead of an empty box.
    if (providers.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'provider-card-item';
        empty.style.cssText = 'display:flex;flex-direction:column;gap:8px;align-items:flex-start';
        const title = document.createElement('span');
        title.className = 'provider-card-name';
        title.textContent = 'No provider configured';
        const hint = document.createElement('span');
        hint.className = 'provider-card-url';
        hint.textContent = 'Add a provider to start chatting — nothing is configured by default.';
        const cta = document.createElement('button');
        cta.className = 'btn-card-action primary';
        cta.textContent = 'Add Provider';
        cta.addEventListener('click', showAddProviderForm);
        empty.append(title, hint, cta);
        configuredProvidersContainer.appendChild(empty);
        return;
    }

    const currentBase = (currentConfig && currentConfig.api_base) || '';
    providers.forEach(p => {
        const card = document.createElement('div');
        const isActive = (currentConfig && currentConfig.provider === p.id)
            || (p.api_base === currentBase && currentConfig.provider === p.id);
        card.className = `provider-card-item ${isActive ? 'active' : ''}`;
        const keyState = p.api_key ? '' : ' · no API key';

        card.innerHTML = `
            <div class="provider-card-meta">
                <div class="provider-card-title-row">
                    <span class="provider-card-name">${escapeHtml(p.name)}</span>
                    ${isActive ? `<span class="badge-active-provider">Active</span>` : ''}
                </div>
                <span class="provider-card-url">${escapeHtml(p.api_base)}</span>
                <span class="provider-card-model">Default: ${escapeHtml(p.default_model || 'auto')}${escapeHtml(keyState)}</span>
            </div>
            <div class="provider-card-actions">
                ${!isActive ? `<button class="btn-card-action primary" data-provider-action="use" data-provider-id="${escapeHtml(p.id)}">Use</button>` : ''}
                <button class="btn-card-action" data-provider-action="test" data-provider-id="${escapeHtml(p.id)}">Test</button>
                <button class="btn-card-action" data-provider-action="edit" data-provider-id="${escapeHtml(p.id)}">Edit</button>
                ${p.id !== 'nvidia' ? `<button class="btn-card-action danger" data-provider-action="delete" data-provider-id="${escapeHtml(p.id)}">Delete</button>` : ''}
            </div>
        `;
        configuredProvidersContainer.appendChild(card);
    });
}

// Delegated handler for provider card actions (avoids inline onclick with
// interpolated values, which was an XSS sink for api_base/api_key).
configuredProvidersContainer.addEventListener('click', (ev) => {
    const btn = ev.target.closest('[data-provider-action]');
    if (!btn) return;
    const id = btn.dataset.providerId;
    const provider = ((currentConfig && currentConfig.custom_providers) || []).find(p => p.id === id);
    switch (btn.dataset.providerAction) {
        case 'use': switchProvider(id); break;
        case 'test':
            if (provider) testSpecificProvider(provider.api_base, provider.api_key);
            break;
        case 'edit': editCustomProvider(id); break;
        case 'delete': deleteCustomProvider(id); break;
    }
});

function showAddProviderForm() {
    providerEditorCard.classList.remove('hidden');
    editProviderName.value = '';
    editProviderId.value = '';
    editProviderBase.value = '';
    editProviderKey.value = '';
    editProviderModel.value = '';
    providerTestStatus.classList.add('hidden');
    document.getElementById('provider-editor-title').textContent = 'Add Custom AI Provider';
    editProviderName.focus();
}

function hideAddProviderForm() {
    providerEditorCard.classList.add('hidden');
}

window.editCustomProvider = function(pId) {
    if (!currentConfig || !currentConfig.custom_providers) return;
    const p = currentConfig.custom_providers.find(x => x.id === pId);
    if (!p) return;
    showAddProviderForm();
    document.getElementById('provider-editor-title').textContent = `Edit Provider: ${p.name}`;
    editProviderName.value = p.name;
    editProviderId.value = p.id;
    editProviderBase.value = p.api_base;
    editProviderKey.value = p.api_key;
    editProviderModel.value = p.default_model;
};

window.deleteCustomProvider = async function(pId) {
    if (!currentConfig || !currentConfig.custom_providers) return;
    const ok = await uiConfirm(`Delete provider '${pId}'?`, {
        title: 'Delete AI Provider',
        confirmText: 'Delete Provider',
        danger: true
    });
    if (ok) {
        currentConfig.custom_providers = currentConfig.custom_providers.filter(x => x.id !== pId);
        saveConfig();
        renderConfiguredProviders();
        showToast(`Deleted provider '${pId}'.`, 'info');
    }
};

window.switchProvider = async function(pId) {
    if (!currentConfig || !currentConfig.custom_providers) return;
    const p = currentConfig.custom_providers.find(x => x.id === pId);
    if (!p) return;

    try {
        const res = await fetch('/api/providers/active', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ provider_id: pId })
        });
        if (!res.ok) {
            // Never swallow this: a 404 means the provider was not persisted.
            throw new Error(res.status === 404
                ? 'Provider not found on the server — save the provider first.'
                : `HTTP ${res.status}`);
        }
        currentConfig = await res.json();
        labelActiveProviderName.textContent = p.name;
        const shortModel = (currentConfig.model || p.default_model).split('/').pop();
        if (labelActiveModel) labelActiveModel.textContent = shortModel;
        if (cfgApiBase) cfgApiBase.value = currentConfig.api_base || '';
        if (cfgApiKey) {
            // Show empty rather than the redaction placeholder; leaving it blank
            // keeps the stored key.
            cfgApiKey.value = (currentConfig.api_key && currentConfig.api_key !== '__harness_redacted__')
                ? currentConfig.api_key : '';
            cfgApiKey.placeholder = currentConfig.api_key === '__harness_redacted__'
                ? 'stored key — leave blank to keep'
                : 'API key';
        }
        if (cfgModel) cfgModel.value = currentConfig.model || '';
        refreshReasoningControlsForModel(currentConfig.model || '', false);

        document.querySelectorAll('.preset-btn').forEach(b => b.classList.remove('active'));
        const btn = document.getElementById(`preset-${p.id}`);
        if (btn) btn.classList.add('active');

        renderConfiguredProviders();
        fetchRemoteModels();
    } catch (e) {
        showToast('Failed to switch provider: ' + e.message, 'error', 7000);
    }
};

async function testProviderConnection() {
    const apiBase = editProviderBase.value.trim();
    const apiKey = editProviderKey.value.trim();
    if (!apiBase) {
        showToast('Please enter an API Base URL.', 'warning');
        return;
    }

    btnTestProviderConnection.textContent = "Testing...";
    providerTestStatus.classList.remove('hidden');
    providerTestStatus.className = 'provider-test-status';
    providerTestStatus.textContent = `Connecting to ${apiBase}/models...`;

    try {
        // POST body keeps the key out of URLs, logs and browser history.
        const res = await fetch('/api/models/remote', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ api_base: apiBase, api_key: apiKey })
        });
        if (!res.ok) throw new Error((await res.text()) || `Status ${res.status}`);
        const models = await res.json();
        providerTestStatus.className = 'provider-test-status success';
        providerTestStatus.textContent = `✓ Connected successfully! Found ${models.length} remote models.`;
        btnTestProviderConnection.textContent = `✓ ${models.length} Models`;
        if (!editProviderModel.value && models.length > 0) editProviderModel.value = models[0].id;
    } catch (e) {
        providerTestStatus.className = 'provider-test-status error';
        providerTestStatus.textContent = `✕ Connection failed: ${e.message}`;
        btnTestProviderConnection.textContent = "Test Failed";
    }
}

window.applyProviderPreset = function(type) {
    if (type === 'nvidia') {
        editProviderName.value = 'NVIDIA NIM';
        editProviderId.value = 'nvidia';
        editProviderBase.value = 'https://integrate.api.nvidia.com/v1';
        editProviderKey.value = DEFAULT_NVIDIA_KEY;
        editProviderModel.value = 'nvidia/nemotron-3-nano-omni-30b-a3b-reasoning';
    } else if (type === 'deepseek') {
        editProviderName.value = 'DeepSeek Official';
        editProviderId.value = 'deepseek';
        editProviderBase.value = 'https://api.deepseek.com/v1';
        editProviderKey.value = '';
        editProviderModel.value = 'deepseek-chat';
    } else if (type === 'openrouter') {
        editProviderName.value = 'OpenRouter';
        editProviderId.value = 'openrouter';
        editProviderBase.value = 'https://openrouter.ai/api/v1';
        editProviderKey.value = '';
        editProviderModel.value = 'deepseek/deepseek-r1';
    } else if (type === 'groq') {
        editProviderName.value = 'Groq Cloud';
        editProviderId.value = 'groq';
        editProviderBase.value = 'https://api.groq.com/openai/v1';
        editProviderKey.value = '';
        editProviderModel.value = 'deepseek-r1-distill-llama-70b';
    } else if (type === 'ollama') {
        editProviderName.value = 'Local Ollama';
        editProviderId.value = 'ollama';
        editProviderBase.value = 'http://localhost:11434/v1';
        editProviderKey.value = 'ollama';
        editProviderModel.value = 'llama3.3';
    } else if (type === 'vllm') {
        editProviderName.value = 'Local vLLM / SGLang';
        editProviderId.value = 'vllm';
        editProviderBase.value = 'http://localhost:8000/v1';
        editProviderKey.value = '';
        editProviderModel.value = 'default';
    }
};

window.testSpecificProvider = async function(base, key) {
    const start = performance.now();
    try {
        const res = await fetch('/api/models/remote', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ api_base: base, api_key: key })
        });
        const lat = Math.round(performance.now() - start);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const models = await res.json();
        showToast(`Provider online (${lat}ms · ${models.length} models)`, 'success', 5000);
    } catch (e) {
        showToast(`Provider unreachable: ${e.message}`, 'error');
    }
};

async function saveCustomProvider() {
    const name = editProviderName.value.trim();
    const id = (editProviderId.value.trim() || name.toLowerCase().replace(/[^a-z0-9]/g, '-'));
    const base = editProviderBase.value.trim();
    const key = editProviderKey.value.trim();
    const defaultModel = editProviderModel.value.trim() || 'default';

    if (!name || !base) {
        showToast('Provider Name and API Base URL are required.', 'warning');
        return;
    }
    if (!currentConfig) return;
    if (!currentConfig.custom_providers) currentConfig.custom_providers = [];

    const existingIdx = currentConfig.custom_providers.findIndex(x => x.id === id);
    const newProv = { id, name, api_base: base, api_key: key, default_model: defaultModel, is_active: true };
    if (existingIdx >= 0) currentConfig.custom_providers[existingIdx] = newProv;
    else currentConfig.custom_providers.push(newProv);

    // Persist the provider BEFORE activating it: `POST /api/providers/active`
    // only knows about providers already stored on the server, so activating
    // first returned 404 and the new provider was silently dropped.
    btnSaveCustomProvider.disabled = true;
    const originalLabel = btnSaveCustomProvider.textContent;
    btnSaveCustomProvider.textContent = 'Saving...';
    try {
        const result = await saveCurrentConfig();
        if (result.ok) {
            renderConfiguredProviders();
            await switchProvider(id);
            showToast(`Provider '${name}' saved.`, 'success');
        } else if (result.status !== 409) {
            throw new Error('HTTP ' + result.status);
        }
    } catch (e) {
        showToast('Failed to save provider: ' + e.message, 'error', 7000);
    } finally {
        btnSaveCustomProvider.disabled = false;
        btnSaveCustomProvider.textContent = originalLabel;
    }
    hideAddProviderForm();
}

// Reasoning Effort & Permission Mode
function setReasoningEffort(effort, shouldClosePopovers = true, persist = true, modelIdOverride = null) {
    const activeModelId = modelIdOverride || (currentConfig && currentConfig.model) || (cfgModel && cfgModel.value.trim()) || '';
    const effective = renderReasoningUiForModel(activeModelId, effort);

    if (persist && currentConfig) {
        currentConfig.reasoning_effort = effective;
        persistCurrentConfigInBackground();
    }
    if (shouldClosePopovers) closeAllPopovers();
}

function setPermissionMode(mode, persist = true) {
    const labels = { 'safe': 'Safe (Ask)', 'autonomous': 'Autonomous', 'read_only': 'Read Only' };
    if (labelPermMode) labelPermMode.textContent = labels[mode] || 'Safe (Ask)';
    if (btnPermModeDropdown) btnPermModeDropdown.className = `permission-mode-pill mode-${mode}`;

    document.querySelectorAll('#perm-mode-popover .popover-item').forEach(el => {
        el.classList.toggle('active', el.getAttribute('data-perm-mode') === mode);
    });

    const permGlobalMode = document.getElementById('perm-global-mode');
    if (permGlobalMode) {
        permGlobalMode.querySelectorAll('.seg-btn').forEach(btn => {
            btn.classList.toggle('active', btn.getAttribute('data-val') === mode);
        });
    }

    if (persist && currentConfig && currentConfig.permissions) {
        // The permission policy is server-owned: `POST /api/config` deliberately
        // ignores it, so writing the pill into `currentConfig` and saving the whole
        // snapshot used to return 200 and change NOTHING - the control lied. It now
        // round-trips through its own endpoint, and the pill follows what the server
        // echoes back rather than what the user clicked.
        (async () => {
            try {
                const res = await fetch('/api/security/permissions', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ mode })
                });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                const saved = await res.json();
                setPermissionMode(saved.permissions.mode, false);
                if (currentConfig.permissions) Object.assign(currentConfig.permissions, saved.permissions);
                showToast('Permission mode set to ' + describePermissionMode(saved.permissions.mode), 'success');
            } catch (e) {
                // Revert: the pill must never claim a mode the server refused.
                setPermissionMode((currentConfig.permissions && currentConfig.permissions.mode) || 'safe', false);
                showToast('Could not change the permission mode: ' + e.message, 'error');
            }
        })();
    }
    closeAllPopovers();
}

// Voice Input & Voice Activity Detection (VAD)
let voiceInputActive = false;
let activeSpeechRecognition = null;
let vadAudioContext = null;
let vadMediaStream = null;
let vadRafId = null;
let vadSilenceTimer = null;

function isVoiceActivityEnabled() {
    if (cfgVoiceActivityDetection) return !!cfgVoiceActivityDetection.checked;
    return Boolean(currentConfig && currentConfig.voice_activity_detection);
}

function toggleVoiceInput() {
    if (voiceInputActive) stopVoiceInput();
    else startVoiceInput();
}

function startVoiceInput() {
    const SpeechRec = window.SpeechRecognition || window.webkitSpeechRecognition;
    if (!SpeechRec) {
        showToast('Voice dictation is not supported in this browser.', 'warning');
        return;
    }
    try {
        const rec = new SpeechRec();
        rec.continuous = true;
        rec.interimResults = true;
        rec.lang = navigator.language || 'en-US';

        const basePrefix = taskInput && taskInput.value.trim()
            ? `${taskInput.value.trimEnd()} `
            : '';
        let committedText = '';

        rec.onresult = (event) => {
            let finalChunk = '';
            let interimChunk = '';
            for (let i = event.resultIndex; i < event.results.length; i++) {
                const transcript = event.results[i][0]?.transcript || '';
                if (event.results[i].isFinal) finalChunk += transcript;
                else interimChunk += transcript;
            }
            if (finalChunk) committedText += finalChunk;
            if (taskInput) {
                taskInput.value = `${basePrefix}${committedText}${interimChunk}`.trimStart();
                autoResizeTextarea();
            }
        };

        rec.onerror = (event) => {
            if (event.error && event.error !== 'aborted' && event.error !== 'no-speech') {
                showToast(`Voice input error: ${event.error}`, 'error');
            }
            stopVoiceInput();
        };

        rec.onend = () => {
            if (voiceInputActive) stopVoiceInput();
        };

        activeSpeechRecognition = rec;
        voiceInputActive = true;
        if (btnMic) {
            btnMic.classList.add('listening');
            btnMic.title = isVoiceActivityEnabled()
                ? 'Listening (VAD active) — click to stop'
                : 'Listening — click to stop';
        }
        rec.start();
        if (isVoiceActivityEnabled()) startVadMonitor();
    } catch (e) {
        stopVoiceInput();
        showToast('Failed to start voice input: ' + e.message, 'error');
    }
}

function stopVoiceInput() {
    voiceInputActive = false;
    stopVadMonitor();
    if (activeSpeechRecognition) {
        try { activeSpeechRecognition.stop(); } catch (_) {}
        activeSpeechRecognition = null;
    }
    if (btnMic) {
        btnMic.classList.remove('listening', 'vad-speaking');
        btnMic.title = 'Voice Input';
    }
}

async function startVadMonitor() {
    stopVadMonitor();
    if (!navigator.mediaDevices || typeof navigator.mediaDevices.getUserMedia !== 'function') return;
    try {
        vadMediaStream = await navigator.mediaDevices.getUserMedia({ audio: true });
        if (!voiceInputActive) {
            stopVadMonitor();
            return;
        }
        const AudioCtx = window.AudioContext || window.webkitAudioContext;
        if (!AudioCtx) return;
        vadAudioContext = new AudioCtx();
        const source = vadAudioContext.createMediaStreamSource(vadMediaStream);
        const analyser = vadAudioContext.createAnalyser();
        analyser.fftSize = 512;
        source.connect(analyser);

        const data = new Uint8Array(analyser.fftSize);
        let sawSpeech = false;

        const tick = () => {
            if (!voiceInputActive || !vadAudioContext) return;
            analyser.getByteTimeDomainData(data);
            let sum = 0;
            for (let i = 0; i < data.length; i++) {
                const v = (data[i] - 128) / 128;
                sum += v * v;
            }
            const rms = Math.sqrt(sum / data.length);
            const isSpeaking = rms > 0.025;
            if (btnMic) btnMic.classList.toggle('vad-speaking', isSpeaking);

            if (isSpeaking) {
                sawSpeech = true;
                if (vadSilenceTimer) {
                    clearTimeout(vadSilenceTimer);
                    vadSilenceTimer = null;
                }
            } else if (sawSpeech && !vadSilenceTimer) {
                vadSilenceTimer = setTimeout(() => {
                    if (voiceInputActive) stopVoiceInput();
                }, 2200);
            }
            vadRafId = requestAnimationFrame(tick);
        };
        vadRafId = requestAnimationFrame(tick);
    } catch (_) {
        stopVadMonitor();
    }
}

function stopVadMonitor() {
    if (vadRafId !== null) {
        cancelAnimationFrame(vadRafId);
        vadRafId = null;
    }
    if (vadSilenceTimer) {
        clearTimeout(vadSilenceTimer);
        vadSilenceTimer = null;
    }
    if (vadAudioContext) {
        try { vadAudioContext.close(); } catch (_) {}
        vadAudioContext = null;
    }
    if (vadMediaStream) {
        vadMediaStream.getTracks().forEach(t => t.stop());
        vadMediaStream = null;
    }
    if (btnMic) btnMic.classList.remove('vad-speaking');
}

// ==========================================================================
// MCP Server Management
// ==========================================================================
async function loadMcpServers() {
    if (!configuredMcpContainer) return;
    try {
        const res = await fetch('/api/mcp');
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const servers = await res.json();
        if (currentConfig) currentConfig.mcp_servers = servers;
        renderMcpServers(servers);
    } catch (e) {
        configuredMcpContainer.innerHTML = `<div class="loading-state">Error loading MCP servers: ${escapeHtml(e.message)}</div>`;
    }
}

function syncGoogleMcpSpotlight(servers) {
    const googleServer = Array.isArray(servers)
        ? servers.find(s => s.id === 'google-ai-search')
        : null;
    if (!googleServer) return;

    const env = googleServer.env || {};
    if (googleMcpEnabled) googleMcpEnabled.checked = !!googleServer.enabled;
    if (googleMcpApiKey) googleMcpApiKey.value = env.GEMINI_API_KEY || '';
    if (googleMcpModel && env.GOOGLE_AI_MODEL) {
        setCustomSelectValue(googleMcpModel, env.GOOGLE_AI_MODEL);
    }
    if (googleMcpStatus) {
        const hasKey = !!(env.GEMINI_API_KEY && env.GEMINI_API_KEY.trim());
        googleMcpStatus.textContent = googleServer.enabled
            ? (hasKey ? 'Enabled · API Key set' : 'Enabled · Missing API key')
            : 'Disabled';
        googleMcpStatus.className = `google-mcp-status${googleServer.enabled && hasKey ? ' online' : ''}`;
    }
}

function buildGoogleMcpPayload() {
    const existing = (currentConfig && Array.isArray(currentConfig.mcp_servers))
        ? currentConfig.mcp_servers.find(s => s.id === 'google-ai-search')
        : null;
    const base = existing ? { ...existing } : {
        id: 'google-ai-search',
        name: 'Google AI Search (MCP)',
        transport: 'builtin',
        command: 'builtin://google-ai-search',
        args: [],
        env: {},
        url: 'https://www.google.com/search',
        enabled: true
    };
    const nextEnv = { ...(base.env || {}) };
    nextEnv.GEMINI_API_KEY = googleMcpApiKey ? googleMcpApiKey.value.trim() : '';
    nextEnv.GOOGLE_AI_MODEL = googleMcpModel ? googleMcpModel.value.trim() || 'gemini-2.5-flash' : 'gemini-2.5-flash';
    base.env = nextEnv;
    base.enabled = googleMcpEnabled ? !!googleMcpEnabled.checked : true;
    return base;
}

async function saveGoogleMcpConfig(silent = false) {
    const payload = buildGoogleMcpPayload();
    if (googleMcpStatus && !silent) {
        googleMcpStatus.textContent = 'Saving...';
        googleMcpStatus.className = 'google-mcp-status';
    }
    try {
        const res = await fetch('/api/mcp', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        await loadMcpServers();
        if (!silent && googleMcpStatus) {
            googleMcpStatus.textContent = payload.enabled ? '✓ Saved & active' : '✓ Saved (disabled)';
            googleMcpStatus.className = `google-mcp-status${payload.enabled ? ' online' : ''}`;
        }
        if (typeof loadTools === 'function') loadTools();
        return true;
    } catch (e) {
        if (googleMcpStatus) {
            googleMcpStatus.textContent = `✕ Save failed: ${e.message}`;
            googleMcpStatus.className = 'google-mcp-status error';
        }
        return false;
    }
}

async function testGoogleMcpServer() {
    const saved = await saveGoogleMcpConfig(true);
    if (!saved) return;
    if (googleMcpStatus) {
        googleMcpStatus.textContent = 'Testing...';
        googleMcpStatus.className = 'google-mcp-status';
    }
    try {
        const res = await fetch('/api/mcp/google-ai-search/test', { method: 'POST' });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const result = await res.json();
        if (result.success) {
            const toolNames = Array.isArray(result.tools) ? result.tools.map(t => t.name).join(', ') : 'search';
            if (googleMcpStatus) {
                googleMcpStatus.textContent = `✓ Online (${result.latency_ms}ms) · ${toolNames}`;
                googleMcpStatus.className = 'google-mcp-status online';
            }
            if (typeof loadTools === 'function') loadTools();
        } else if (googleMcpStatus) {
            googleMcpStatus.textContent = `✕ ${result.error || 'Connection failed'}`;
            googleMcpStatus.className = 'google-mcp-status error';
        }
    } catch (e) {
        if (googleMcpStatus) {
            googleMcpStatus.textContent = `✕ ${e.message}`;
            googleMcpStatus.className = 'google-mcp-status error';
        }
    }
}

function renderMcpServers(servers) {
    syncGoogleMcpSpotlight(servers);
    if (!configuredMcpContainer) return;
    configuredMcpContainer.innerHTML = '';

    const customServers = Array.isArray(servers)
        ? servers.filter(s => s.id !== 'google-ai-search')
        : [];

    if (customServers.length === 0) {
        configuredMcpContainer.innerHTML = `<div class="loading-state">No external stdio/SSE MCP servers configured. Click a preset or + Add Custom Server above.</div>`;
        return;
    }

    customServers.forEach(s => {
        const card = document.createElement('div');
        card.id = `mcp-card-${s.id}`;
        card.className = `mcp-server-card ${s.enabled ? 'enabled' : ''}`;
        const cmdPreview = s.transport === 'sse'
            ? s.url
            : (s.transport === 'builtin' ? (s.command || s.url || 'builtin') : `${s.command} ${(s.args || []).join(' ')}`);

        card.innerHTML = `
            <div class="mcp-server-head">
                <div class="mcp-server-title-wrap">
                    <span class="mcp-server-name">${escapeHtml(s.name)}</span>
                    <span class="mcp-transport-badge">${escapeHtml(s.transport || 'stdio')}</span>
                </div>
                <div class="mcp-status-pill" id="mcp-status-${s.id}">
                    <span class="dot-live"></span>
                    <span>${s.enabled ? 'Enabled' : 'Disabled'}</span>
                </div>
            </div>
            <div class="mcp-command-preview">${escapeHtml(cmdPreview)}</div>
            <div class="mcp-tools-section" id="mcp-tools-${s.id}"></div>
            <div class="mcp-server-actions">
                <label class="mcp-toggle-label ui-switch-row">
                    <span class="ui-switch compact">
                        <input type="checkbox" ${s.enabled ? 'checked' : ''} data-mcp-toggle="${s.id}">
                        <span class="ui-switch-track"><span class="ui-switch-thumb"></span></span>
                    </span>
                    <span>Active in Toolset</span>
                </label>
                <div class="mcp-btns-group">
                    <button class="btn-card-action primary" data-mcp-action="test" data-mcp-id="${s.id}">⚡ Test Handshake</button>
                    <button class="btn-card-action" data-mcp-action="edit" data-mcp-id="${s.id}">Edit</button>
                    <button class="btn-card-action danger" data-mcp-action="delete" data-mcp-id="${s.id}">Remove</button>
                </div>
            </div>
        `;
        configuredMcpContainer.appendChild(card);
    });
}

function showAddMcpForm() {
    mcpEditorCard.classList.remove('hidden');
    editMcpName.value = '';
    editMcpId.value = '';
    setCustomSelectValue(editMcpTransport, 'stdio');
    editMcpCommand.value = '';
    editMcpArgs.value = '';
    editMcpUrl.value = '';
    editMcpEnv.value = '';
    mcpTestStatus.classList.add('hidden');
    document.getElementById('mcp-editor-title').textContent = 'Configure MCP Server';
    const cWrap = document.getElementById('mcp-command-wrap');
    const aWrap = document.getElementById('mcp-args-wrap');
    const uWrap = document.getElementById('mcp-url-wrap');
    if (cWrap) cWrap.classList.remove('hidden');
    if (aWrap) aWrap.classList.remove('hidden');
    if (uWrap) uWrap.classList.add('hidden');
    editMcpName.focus();
}

function hideAddMcpForm() {
    mcpEditorCard.classList.add('hidden');
}

window.editMcpServer = function(id) {
    const servers = (currentConfig && currentConfig.mcp_servers) || [];
    const s = servers.find(x => x.id === id);
    if (!s) return;

    showAddMcpForm();
    document.getElementById('mcp-editor-title').textContent = `Edit MCP Server: ${s.name}`;
    editMcpName.value = s.name;
    editMcpId.value = s.id;
    setCustomSelectValue(editMcpTransport, s.transport || 'stdio');
    editMcpCommand.value = s.command || '';
    editMcpArgs.value = (s.args || []).join(' ');
    editMcpUrl.value = s.url || '';
    editMcpEnv.value = Object.entries(s.env || {}).map(([k, v]) => `${k}=${v}`).join(', ');

    const isSse = s.transport === 'sse';
    const cWrap = document.getElementById('mcp-command-wrap');
    const aWrap = document.getElementById('mcp-args-wrap');
    const uWrap = document.getElementById('mcp-url-wrap');
    if (cWrap) cWrap.classList.toggle('hidden', isSse);
    if (aWrap) aWrap.classList.toggle('hidden', isSse);
    if (uWrap) uWrap.classList.toggle('hidden', !isSse);
};

window.applyMcpPreset = function(preset) {
    showAddMcpForm();
    if (preset === 'memory') {
        editMcpName.value = 'Knowledge Graph Memory (MCP)';
        editMcpId.value = 'memory';
        setCustomSelectValue(editMcpTransport, 'stdio');
        editMcpCommand.value = 'npx';
        editMcpArgs.value = '-y @modelcontextprotocol/server-memory';
    } else if (preset === 'fs') {
        editMcpName.value = 'Filesystem Explorer (MCP)';
        editMcpId.value = 'filesystem';
        setCustomSelectValue(editMcpTransport, 'stdio');
        editMcpCommand.value = 'npx';
        editMcpArgs.value = '-y @modelcontextprotocol/server-filesystem .';
    } else if (preset === 'sqlite') {
        editMcpName.value = 'SQLite Database (MCP)';
        editMcpId.value = 'sqlite';
        setCustomSelectValue(editMcpTransport, 'stdio');
        editMcpCommand.value = 'uvx';
        editMcpArgs.value = 'mcp-server-sqlite --db-path ./harness.db';
    } else if (preset === 'git') {
        editMcpName.value = 'Git Repository Tools (MCP)';
        editMcpId.value = 'git';
        setCustomSelectValue(editMcpTransport, 'stdio');
        editMcpCommand.value = 'uvx';
        editMcpArgs.value = 'mcp-server-git';
    } else if (preset === 'fetch') {
        editMcpName.value = 'Web Fetch & Extract (MCP)';
        editMcpId.value = 'fetch';
        setCustomSelectValue(editMcpTransport, 'stdio');
        editMcpCommand.value = 'uvx';
        editMcpArgs.value = 'mcp-server-fetch';
    }
};

async function saveMcpServer() {
    const name = editMcpName.value.trim();
    const id = editMcpId.value.trim() || name.toLowerCase().replace(/[^a-z0-9]/g, '_');
    const transport = editMcpTransport.value;
    const command = editMcpCommand.value.trim();
    const args = editMcpArgs.value.trim() ? editMcpArgs.value.trim().split(/\s+/) : [];
    const url = editMcpUrl.value.trim();

    if (!name || (!command && !url)) {
        showToast('Server name and command (or URL) are required.', 'warning');
        return;
    }

    const env = {};
    const rawEnv = editMcpEnv.value.trim();
    if (rawEnv) {
        rawEnv.split(',').forEach(pair => {
            const [k, v] = pair.split('=').map(s => s.trim());
            if (k) env[k] = v || '';
        });
    }

    try {
        const res = await fetch('/api/mcp', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ id, name, transport, command, args, env, url, enabled: true })
        });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        hideAddMcpForm();
        loadMcpServers();
        showToast(`Saved MCP server '${name}'.`, 'success');
    } catch (e) {
        showToast('Failed to save MCP server: ' + e.message, 'error');
    }
}

window.toggleMcpServer = async function(id, enabled) {
    if (!currentConfig || !currentConfig.mcp_servers) return;
    const s = currentConfig.mcp_servers.find(x => x.id === id);
    if (!s) return;
    s.enabled = enabled;
    try {
        await fetch('/api/mcp', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(s)
        });
        loadMcpServers();
    } catch (e) {
        showToast('Failed to toggle MCP server: ' + e.message, 'error');
    }
};

window.deleteMcpServer = async function(id) {
    const ok = await uiConfirm(`Delete MCP server '${id}'?`, {
        title: 'Delete MCP Server',
        confirmText: 'Remove Server',
        danger: true
    });
    if (!ok) return;
    try {
        const res = await fetch(`/api/mcp/${id}`, { method: 'DELETE' });
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        loadMcpServers();
        showToast(`Removed MCP server '${id}'.`, 'info');
    } catch (e) {
        showToast('Failed to delete MCP server: ' + e.message, 'error');
    }
};

window.testMcpServer = async function(id) {
    const statusPill = document.getElementById(`mcp-status-${id}`);
    const toolsBox = document.getElementById(`mcp-tools-${id}`);
    if (statusPill) statusPill.innerHTML = `<span class="pulse-dot"></span><span>Connecting...</span>`;

    try {
        const res = await fetch(`/api/mcp/${id}/test`, { method: 'POST' });
        if (!res.ok) throw new Error(`Status ${res.status}`);
        const result = await res.json();

        if (result.success) {
            if (statusPill) {
                statusPill.className = 'mcp-status-pill online';
                statusPill.innerHTML = `<span class="dot-live"></span><span>Online (${result.latency_ms}ms)</span>`;
            }
            if (toolsBox) {
                toolsBox.innerHTML = `
                    <div class="mcp-tools-label">Discovered Tools (${result.tools.length}):</div>
                    <div class="mcp-tools-chips">
                        ${result.tools.map(t => `<span class="mcp-tool-chip" title="${escapeHtml(t.description)}">${escapeHtml(t.name)}</span>`).join('')}
                    </div>
                `;
            }
        } else {
            if (statusPill) {
                statusPill.className = 'mcp-status-pill';
                statusPill.innerHTML = `<span>✕ Error</span>`;
            }
            if (toolsBox) {
                toolsBox.innerHTML = `<div style="color: var(--accent-red); font-size: 0.74rem;">Error: ${escapeHtml(result.error || 'Connection failed')}</div>`;
            }
        }
    } catch (e) {
        if (statusPill) statusPill.innerHTML = `<span>✕ Failed</span>`;
        if (toolsBox) toolsBox.innerHTML = `<div style="color: var(--accent-red); font-size: 0.74rem;">${escapeHtml(e.message)}</div>`;
    }
};

async function testMcpServerForm() {
    btnTestMcpServer.textContent = "Testing...";
    mcpTestStatus.classList.remove('hidden');
    mcpTestStatus.className = 'provider-test-status';
    mcpTestStatus.textContent = "Launching MCP server subprocess and testing handshake...";

    const id = editMcpId.value.trim() || 'test';
    const payload = {
        id,
        name: editMcpName.value.trim() || 'Test Server',
        transport: editMcpTransport.value,
        command: editMcpCommand.value.trim(),
        args: editMcpArgs.value.trim() ? editMcpArgs.value.trim().split(/\s+/) : [],
        env: {},
        url: editMcpUrl.value.trim(),
        enabled: true
    };

    try {
        const saveRes = await fetch('/api/mcp', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        if (!saveRes.ok) throw new Error("Save error");
        const testRes = await fetch(`/api/mcp/${id}/test`, { method: 'POST' });
        const res = await testRes.json();

        if (res.success) {
            mcpTestStatus.className = 'provider-test-status success';
            mcpTestStatus.textContent = `✓ Handshake succeeded in ${res.latency_ms}ms! Discovered ${res.tools.length} tool(s): ${res.tools.map(t => t.name).join(', ')}`;
            btnTestMcpServer.textContent = `✓ ${res.tools.length} Tools`;
        } else {
            mcpTestStatus.className = 'provider-test-status error';
            mcpTestStatus.textContent = `✕ Handshake failed: ${res.error}`;
            btnTestMcpServer.textContent = "Test Failed";
        }
    } catch (e) {
        mcpTestStatus.className = 'provider-test-status error';
        mcpTestStatus.textContent = `✕ Error: ${e.message}`;
        btnTestMcpServer.textContent = "Test Failed";
    }
}

// ==========================================================================
// Search Engines & Providers Management
// ==========================================================================
function syncSearchModalUI() {
    if (!currentConfig || !currentConfig.search) return;
    const s = currentConfig.search;
    selectSearchProvider(s.active_provider || 'tinyfish', false);
    if (cfgSearchMaxResults) cfgSearchMaxResults.value = s.max_results || 5;
    if (cfgSearxngUrl) cfgSearxngUrl.value = s.searxng_url || '';
}

function selectSearchProvider(providerId, updateConfigObj = true) {
    document.querySelectorAll('.search-provider-card').forEach(card => {
        card.classList.toggle('active', card.getAttribute('data-search-provider') === providerId);
    });

    const titleMap = {
        'tinyfish': 'Configure TinyFish Search',
        'duckduckgo': 'DuckDuckGo Instant Search (Free)',
        'tavily': 'Configure Tavily Search',
        'brave': 'Configure Brave Search',
        'searxng': 'Configure SearXNG Search'
    };
    if (searchSettingsTitle) searchSettingsTitle.textContent = titleMap[providerId] || 'Configure Search Provider';

    const keyWrap = document.getElementById('search-key-wrap');
    const searxngWrap = document.getElementById('search-searxng-url-wrap');

    if (providerId === 'duckduckgo') {
        if (keyWrap) keyWrap.classList.add('hidden');
        if (searxngWrap) searxngWrap.classList.add('hidden');
    } else if (providerId === 'searxng') {
        if (keyWrap) keyWrap.classList.add('hidden');
        if (searxngWrap) searxngWrap.classList.remove('hidden');
    } else {
        if (keyWrap) keyWrap.classList.remove('hidden');
        if (searxngWrap) searxngWrap.classList.add('hidden');
        if (currentConfig && currentConfig.search && cfgSearchApiKey) {
            if (providerId === 'tinyfish') {
                cfgSearchApiKey.value = currentConfig.search.tinyfish_api_key || currentConfig.tinyfish_api_key || '';
                cfgSearchApiKey.placeholder = "sk-tinyfish-...";
            } else if (providerId === 'tavily') {
                cfgSearchApiKey.value = currentConfig.search.tavily_api_key || '';
                cfgSearchApiKey.placeholder = "tvly-...";
            } else if (providerId === 'brave') {
                cfgSearchApiKey.value = currentConfig.search.brave_api_key || '';
                cfgSearchApiKey.placeholder = "BSA...";
            }
        }
    }

    if (updateConfigObj && currentConfig && currentConfig.search) {
        currentConfig.search.active_provider = providerId;
    }
}

async function saveSearchConfig() {
    if (!currentConfig) return;
    if (!currentConfig.search) currentConfig.search = {};

    const activeCard = document.querySelector('.search-provider-card.active');
    const activeProvider = activeCard ? activeCard.getAttribute('data-search-provider') : 'tinyfish';

    currentConfig.search.active_provider = activeProvider;
    currentConfig.search.max_results = parseInt(cfgSearchMaxResults.value, 10) || 5;

    if (activeProvider === 'tinyfish') {
        currentConfig.search.tinyfish_api_key = cfgSearchApiKey.value.trim();
        currentConfig.tinyfish_api_key = cfgSearchApiKey.value.trim();
    } else if (activeProvider === 'tavily') {
        currentConfig.search.tavily_api_key = cfgSearchApiKey.value.trim();
    } else if (activeProvider === 'brave') {
        currentConfig.search.brave_api_key = cfgSearchApiKey.value.trim();
    } else if (activeProvider === 'searxng') {
        currentConfig.search.searxng_url = cfgSearxngUrl.value.trim();
    }

    try {
        const result = await saveCurrentConfig();
        if (result.ok) {
            showSettingsFeedback(`✓ Search provider '${activeProvider}' saved`);
        }
    } catch (e) {
        showSettingsFeedback(`✕ Failed to save search config: ${e.message}`, true);
    }
}

async function runSearchTest() {
    const query = searchTestQuery.value.trim();
    if (!query) {
        showToast('Please enter a search query to test.', 'warning');
        return;
    }

    btnRunSearchTest.textContent = "Searching...";
    searchTestOutputBox.classList.remove('hidden');
    testOutputProviderBadge.textContent = "Provider: " + (currentConfig?.search?.active_provider || 'Active');
    testOutputLatency.textContent = "Measuring...";
    testOutputRendered.innerHTML = "<em>Sending query to live search engine...</em>";

    const startTime = performance.now();
    try {
        const res = await fetch('/api/search/test', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ query })
        });
        const duration = Math.round(performance.now() - startTime);
        testOutputLatency.textContent = `${duration} ms`;
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const data = await res.json();

        testOutputProviderBadge.textContent = `Provider: ${data.provider}`;
        testOutputRendered.innerHTML = renderMarkdown(data.output);
        btnRunSearchTest.textContent = "Test Search";
    } catch (e) {
        testOutputLatency.textContent = "Error";
        testOutputRendered.innerHTML = `<span style="color: var(--accent-red)">Search test failed: ${escapeHtml(e.message)}</span>`;
        btnRunSearchTest.textContent = "Test Search";
    }
}

// ==========================================================================
// Config, Permissions & Tools
// ==========================================================================
function showSettingsFeedback(message, isError = false) {
    if (!settingsSaveFeedback) return;
    settingsSaveFeedback.textContent = message;
    settingsSaveFeedback.style.color = isError ? 'var(--accent-red)' : 'var(--accent-green)';
    clearTimeout(settingsSaveFeedback._timer);
    settingsSaveFeedback._timer = setTimeout(() => {
        if (settingsSaveFeedback) settingsSaveFeedback.textContent = '';
    }, 3500);
}

async function loadConfig() {
    try {
        const res = await fetch('/api/config');
        if (!res.ok) return;
        currentConfig = await res.json();
        // The mock switch is a UI-local toggle, but the SERVER's mock_mode is the
        // source of truth: without this, enabling mock mode in Settings (or via
        // the API) has no effect and every conversation still hits a real model.
        mockModeEnabled = currentConfig.mock_mode === true;
        const optMockStatus = document.getElementById('opt-mock-status');
        if (optMockStatus) optMockStatus.textContent = mockModeEnabled ? 'On' : 'Off';

        if (cfgApiBase) cfgApiBase.value = currentConfig.api_base || '';
        if (cfgApiKey) cfgApiKey.value = currentConfig.api_key || '';
        if (cfgModel) cfgModel.value = currentConfig.model || '';
        if (cfgMaxOut) cfgMaxOut.value = currentConfig.max_tool_output_bytes || 16384;
        if (cfgSandbox) cfgSandbox.value = currentConfig.sandbox_root || '';

        const mem = currentConfig.memory || {};
        if (cfgAutoDetectContext) cfgAutoDetectContext.checked = mem.auto_detect_context !== false;
        if (cfgMaxContextTokens) cfgMaxContextTokens.value = mem.max_context_tokens ?? 128000;
        if (cfgAutoCompactPct) cfgAutoCompactPct.value = mem.auto_compact_threshold_pct ?? 78;
        if (cfgPreserveRecent) cfgPreserveRecent.value = mem.preserve_recent_messages ?? 8;
        if (cfgPruneToolBytes) cfgPruneToolBytes.value = mem.prune_tool_outputs_over_bytes ?? 2048;
        if (cfgMaxToolRetries) cfgMaxToolRetries.value = mem.max_tool_retries ?? 3;
        if (cfgEnableWorkingMemory) cfgEnableWorkingMemory.checked = mem.enable_working_memory !== false;
        if (cfgSummarizeCompactedTurns) cfgSummarizeCompactedTurns.checked = mem.summarize_compacted_turns !== false;
        if (cfgVoiceActivityDetection) cfgVoiceActivityDetection.checked = Boolean(currentConfig.voice_activity_detection);
        if (cfgAutoUpdateCheck) cfgAutoUpdateCheck.checked = currentConfig.auto_update_check !== false;
        if (cfgAutoUpdateApply) cfgAutoUpdateApply.checked = currentConfig.auto_update_apply !== false;
        if (cfgUpdatePin) cfgUpdatePin.value = currentConfig.update_pin_version || '';
        syncPrivilegedShellUi(currentConfig.allow_privileged_shell === true);
        // Automatic skill routing (defaults on).
        if (cfgAutoSkillRouter) cfgAutoSkillRouter.checked = currentConfig.auto_skill_router !== false;
        if (cfgSkillRouterConfidence) {
            const conf = typeof currentConfig.skill_router_min_confidence === 'number'
                ? currentConfig.skill_router_min_confidence
                : 0.35;
            cfgSkillRouterConfidence.value = conf;
        }
        if (cfgSkillRouterMax) {
            const max = typeof currentConfig.skill_router_max_skills === 'number'
                ? currentConfig.skill_router_max_skills
                : 2;
            cfgSkillRouterMax.value = max;
        }
        const runningVersionEl = document.getElementById('cfg-running-version');
        if (runningVersionEl) {
            runningVersionEl.textContent = 'v' + (updateStatus.current_version || '?');
        }

        syncAutoContextUiState();
        const maxCtx = mem.max_context_tokens || 128000;
        applyResolvedContextWindow(maxCtx);
        if (mem.auto_detect_context !== false && currentConfig.model) {
            syncModelContextWindowIfAuto(currentConfig.model);
        }

        const shortName = (currentConfig.model || 'Nemotron-3-Nano').split('/').pop();
        if (labelActiveModel) labelActiveModel.textContent = shortName;

        const currentProv = currentConfig.custom_providers ? currentConfig.custom_providers.find(p => p.api_base === currentConfig.api_base || p.id === currentConfig.provider) : null;
        if (currentProv && labelActiveProviderName) labelActiveProviderName.textContent = currentProv.name;

        setReasoningEffort(currentConfig.reasoning_effort || 'high', false, false);
        refreshReasoningControlsForModel(currentConfig.model || '', false);
        setPermissionMode((currentConfig.permissions && currentConfig.permissions.mode) || 'safe', false);
        setDisplayVerbosity(currentConfig.display_verbosity || 'compact', false);
        setSendMode(currentConfig.send_mode || 'queue', false);

        allServers = Array.isArray(currentConfig.servers) ? currentConfig.servers : [];
        updateModelPillDisplay();
        renderServerPopover();
        renderConfiguredServers();
        syncOrchestratorControls();
        renderConfiguredProviders();
        renderFilteredModels();
        syncPermissionControls();
        if (Array.isArray(currentConfig.mcp_servers)) {
            syncGoogleMcpSpotlight(currentConfig.mcp_servers);
        }
        fetchRemoteModels();
        probeAllServers();
    } catch (_) {}
}

// ==========================================================================
// Orchestrator Mode, Multi-Profile Subagents & Server Node Management
// ==========================================================================

/// Shared `POST /api/config` path.
///
/// The server owns a monotonic `config_revision` that every mutating endpoint
/// bumps (activate provider, save MCP server, save server, save automation,
/// save skill, ...). A snapshot taken before one of those ran is rejected with
/// **409** so it cannot silently revert it. Without this helper the 409 fell
/// into a generic "HTTP 409" line and the user's edit vanished with no
/// explanation; now the live config is reloaded into the forms and the reason
/// is spelled out.
async function saveCurrentConfig() {
    if (!currentConfig) throw new Error('Configuration is still loading.');
    let res;
    try {
        res = await fetch('/api/config', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(currentConfig)
        });
    } catch (e) {
        showToast('Could not save settings: ' + e.message, 'error', 7000);
        return { ok: false, status: 0, stale: false };
    }

    if (res.ok) {
        const fresh = await res.json().catch(() => null);
        // The response carries the new revision, so the next save is not stale.
        if (fresh) currentConfig = fresh;
        return { ok: true, status: res.status, stale: false };
    }

    const bodyText = await res.text().catch(() => '');
    if (res.status === 409) {
        showToast(
            'Settings changed in another tab or view since this one loaded, so this save was refused ' +
            'to avoid reverting them. The live settings are reloaded — re-apply your change and save again.',
            'warning', 10000
        );
        // Drop the stale snapshot and re-render every settings form from the
        // server's current config.
        await loadConfig();
        return { ok: false, status: 409, stale: true };
    }
    showToast('Could not save settings (HTTP ' + res.status + '): ' + (bodyText || 'no details'), 'error', 7000);
    return { ok: false, status: res.status, stale: false };
}

// Persist the config without blocking the caller; a 409 is still surfaced.
function persistCurrentConfigInBackground() {
    Promise.resolve().then(saveCurrentConfig).catch(() => {});
}

const DEFAULT_ORCHESTRATOR_PROMPT = 'You are operating in Orchestrator Mode.\n- Decompose complex requests into clear, independent workstreams and delegate execution cleanly using `invoke_subagent` (and `swarm_board` for coordination).\n- Choose specialized subagent types when configured, pass `read_only: true` for verification/review agents, and assign remote `server_id` nodes when offloading onto worker servers.\n- Synthesize verified results cleanly for the user without repeating redundant raw logs.';

function getServerSelectOptionsHtml(selectedId = 'local') {
    const servers = (currentConfig && currentConfig.servers) || [];
    const opts = [
        { value: 'local', label: 'Local Server (Default)' },
        { value: 'auto', label: 'Auto / Round-Robin Remote' }
    ];
    servers.filter(s => s.id !== 'local').forEach(s => {
        opts.push({ value: s.id, label: `${s.name} (${s.id})` });
    });
    return opts.map(o => `<option value="${escapeHtml(o.value)}"${o.value === selectedId ? ' selected' : ''}>${escapeHtml(o.label)}</option>`).join('');
}

function getProviderNameLabel(provId) {
    if (!provId) return 'Resolved automatically from model';
    const p = (currentConfig && currentConfig.custom_providers || []).find(x => x.id === provId);
    return `Provider: ${p ? p.name : provId}`;
}

function updateOrchPickerLabels() {
    const activeModel = draftOrchState.model || (currentConfig && currentConfig.model) || 'Follow Active Model';
    const orchShort = draftOrchState.model ? draftOrchState.model.split('/').pop() : `Follow (${activeModel.split('/').pop()})`;
    if (labelOrchModel) labelOrchModel.textContent = orchShort;
    if (labelOrchEffort) labelOrchEffort.textContent = formatReasoningLevelLabel(draftOrchState.thinking_effort || 'high');
    if (labelOrchProviderHint) labelOrchProviderHint.textContent = getProviderNameLabel(draftOrchState.provider_id);

    const subModel = draftOrchState.subagent_model;
    if (labelSubagentModel) labelSubagentModel.textContent = subModel ? subModel.split('/').pop() : 'Inherit Orchestrator';
    if (labelSubagentEffort) {
        const orchEffort = draftOrchState.thinking_effort || (currentConfig && currentConfig.reasoning_effort) || 'high';
        const resolved = resolveSubagentEffort(draftOrchState.subagent_thinking_effort, orchEffort);
        labelSubagentEffort.textContent = formatReasoningLevelLabel ? formatReasoningLevelLabel(resolved) : resolved;
    }
    if (labelSubagentProviderHint) labelSubagentProviderHint.textContent = getProviderNameLabel(draftOrchState.subagent_provider_id);
    if (typeof syncSubagentEffortControls === 'function') syncSubagentEffortControls();
}

function syncSubagentStrategyVisibility() {
    const mode = draftOrchState.subagent_mode === 'multi' ? 'multi' : 'single';
    if (orchSubagentModeCtrl) {
        orchSubagentModeCtrl.querySelectorAll('[data-subagent-mode]').forEach(b => {
            b.classList.toggle('active', b.getAttribute('data-subagent-mode') === mode);
        });
    }
    if (orchSingleSubagentPanel) orchSingleSubagentPanel.classList.toggle('hidden', mode !== 'single');
    if (orchMultiSubagentPanel) orchMultiSubagentPanel.classList.toggle('hidden', mode !== 'multi');
    if (mode === 'multi') renderSubagentProfiles();
}

function renderSubagentProfiles() {
    if (!subagentProfilesContainer) return;
    const profiles = Array.isArray(draftOrchState.subagent_profiles) ? draftOrchState.subagent_profiles : [];
    subagentProfilesContainer.innerHTML = '';

    if (profiles.length === 0) {
        subagentProfilesContainer.innerHTML = '<div style="padding:10px;color:var(--text-dim);font-size:0.78rem;">No subagent types defined yet. Click + Add Subagent Type.</div>';
        return;
    }

    profiles.forEach((prof, idx) => {
        const card = document.createElement('div');
        card.className = 'subagent-profile-card';
        const mShort = prof.model ? prof.model.split('/').pop() : 'Inherit Lead Model';
        const eLabel = formatReasoningLevelLabel(prof.thinking_effort || 'medium');
        card.innerHTML = `
            <div class="subagent-profile-head">
                <input type="text" class="form-input" data-prof-name="${idx}" value="${escapeHtml(prof.name || '')}" placeholder="Type name (e.g. Research, Coding, Reviewer)" style="flex:1;padding:5px 8px;font-size:0.8rem;">
                <input type="text" class="form-input" data-prof-id="${idx}" value="${escapeHtml(prof.id || '')}" placeholder="slug (research)" style="width:110px;padding:5px 8px;font-size:0.76rem;font-family:var(--font-mono);">
                <button type="button" class="btn-card-action danger" data-prof-del="${idx}" title="Delete subagent type">✕</button>
            </div>
            <input type="text" class="form-input" data-prof-desc="${idx}" value="${escapeHtml(prof.description || '')}" placeholder="When the orchestrator should use this subagent type..." style="padding:5px 8px;font-size:0.76rem;">
            <div class="subagent-profile-grid">
                <button type="button" class="model-picker-pill orch-model-picker-pill" data-prof-model-btn="${idx}" title="${escapeHtml(getProviderNameLabel(prof.provider_id))}">
                    <span class="model-name">${escapeHtml(mShort)}</span>
                    <span class="model-effort-inline">${escapeHtml(eLabel)}</span>
                    <span class="arrow-down">⌵</span>
                </button>
                <select class="form-input" data-prof-server="${idx}" style="padding:5px 8px;font-size:0.76rem;">
                    ${getServerSelectOptionsHtml(prof.server_id || 'local')}
                </select>
                <label class="subagent-ro-toggle" title="Restrict this subagent type to read-only inspection">
                    <input type="checkbox" data-prof-ro="${idx}" ${prof.read_only ? 'checked' : ''}>
                    <span>Read-Only</span>
                </label>
            </div>
        `;
        card.querySelector(`[data-prof-name="${idx}"]`)?.addEventListener('input', (e) => {
            prof.name = e.target.value;
            if (!prof._idEdited) {
                prof.id = prof.name.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-|-$/g, '') || `type-${idx + 1}`;
                const idIn = card.querySelector(`[data-prof-id="${idx}"]`);
                if (idIn) idIn.value = prof.id;
            }
        });
        card.querySelector(`[data-prof-id="${idx}"]`)?.addEventListener('input', (e) => {
            prof._idEdited = true;
            prof.id = e.target.value.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '-');
        });
        card.querySelector(`[data-prof-desc="${idx}"]`)?.addEventListener('input', (e) => {
            prof.description = e.target.value;
        });
        card.querySelector(`[data-prof-server="${idx}"]`)?.addEventListener('change', (e) => {
            prof.server_id = e.target.value;
        });
        card.querySelector(`[data-prof-ro="${idx}"]`)?.addEventListener('change', (e) => {
            prof.read_only = !!e.target.checked;
        });
        card.querySelector(`[data-prof-model-btn="${idx}"]`)?.addEventListener('click', (e) => {
            e.stopPropagation();
            const btn = e.currentTarget;
            openUnifiedModelPicker(btn, {
                title: `${prof.name || 'Subagent'} Model`,
                hideOrchestrator: true,
                selectedModel: prof.model || draftOrchState.model || (currentConfig && currentConfig.model) || '',
                selectedProviderId: prof.provider_id || draftOrchState.provider_id || '',
                selectedEffort: prof.thinking_effort || 'medium',
                onSelect: (sel) => {
                    prof.model = sel.model;
                    prof.provider_id = sel.provider_id;
                    if (sel.thinking_effort) prof.thinking_effort = sel.thinking_effort;
                    renderSubagentProfiles();
                }
            });
        });
        card.querySelector(`[data-prof-del="${idx}"]`)?.addEventListener('click', () => {
            draftOrchState.subagent_profiles.splice(idx, 1);
            renderSubagentProfiles();
        });
        subagentProfilesContainer.appendChild(card);
    });
}

function addSubagentProfile() {
    if (!Array.isArray(draftOrchState.subagent_profiles)) draftOrchState.subagent_profiles = [];
    const n = draftOrchState.subagent_profiles.length + 1;
    draftOrchState.subagent_profiles.push({
        id: `custom-${n}`,
        name: `Custom Agent ${n}`,
        description: 'Specialized subagent role',
        provider_id: draftOrchState.subagent_provider_id || draftOrchState.provider_id || '',
        model: draftOrchState.subagent_model || '',
        thinking_effort: draftOrchState.subagent_thinking_effort || 'medium',
        server_id: 'local',
        read_only: false
    });
    draftOrchState.subagent_mode = 'multi';
    syncSubagentStrategyVisibility();
}

function syncOrchestratorControls() {
    if (!currentConfig) return;
    const orch = currentConfig.orchestrator || {};
    draftOrchState = {
        enabled: Boolean(orch.enabled),
        provider_id: orch.provider_id || '',
        model: orch.model || '',
        thinking_effort: orch.thinking_effort || 'high',
        subagent_mode: orch.subagent_mode || 'single',
        subagent_provider_id: orch.subagent_provider_id || '',
        subagent_model: orch.subagent_model || '',
        subagent_thinking_effort: orch.subagent_thinking_effort || 'step_down',
        subagent_server_id: orch.subagent_server_id || 'local',
        subagent_profiles: Array.isArray(orch.subagent_profiles)
            ? JSON.parse(JSON.stringify(orch.subagent_profiles))
            : []
    };

    if (orchEnabled) orchEnabled.checked = draftOrchState.enabled;
    if (orchSystemPrompt) orchSystemPrompt.value = orch.system_prompt || DEFAULT_ORCHESTRATOR_PROMPT;
    if (orchSubagentServer) {
        orchSubagentServer.innerHTML = getServerSelectOptionsHtml(draftOrchState.subagent_server_id || 'local');
    }
    updateOrchPickerLabels();
    syncSubagentStrategyVisibility();
}

async function saveOrchestratorConfig() {
    if (!currentConfig) return;
    const cleanedProfiles = (draftOrchState.subagent_profiles || []).map((p, idx) => ({
        id: (p.id || '').trim() || `worker-${idx + 1}`,
        name: (p.name || '').trim() || `Worker ${idx + 1}`,
        description: (p.description || '').trim(),
        provider_id: p.provider_id || '',
        model: (p.model || '').trim(),
        thinking_effort: p.thinking_effort || 'medium',
        server_id: p.server_id || 'local',
        read_only: Boolean(p.read_only)
    }));

    currentConfig.orchestrator = {
        enabled: orchEnabled ? !!orchEnabled.checked : false,
        provider_id: draftOrchState.provider_id || '',
        model: draftOrchState.model || '',
        thinking_effort: draftOrchState.thinking_effort || 'high',
        subagent_mode: draftOrchState.subagent_mode || 'single',
        subagent_provider_id: draftOrchState.subagent_provider_id || '',
        subagent_model: draftOrchState.subagent_model || '',
        subagent_thinking_effort: draftOrchState.subagent_thinking_effort || 'step_down',
        subagent_server_id: orchSubagentServer ? orchSubagentServer.value : (draftOrchState.subagent_server_id || 'local'),
        subagent_profiles: cleanedProfiles,
        system_prompt: orchSystemPrompt ? orchSystemPrompt.value : DEFAULT_ORCHESTRATOR_PROMPT
    };
    try {
        const result = await saveCurrentConfig();
        if (result.ok) {
            updateModelPillDisplay();
            renderFilteredModels(true);
            showSettingsFeedback('✓ Orchestrator & subagent settings saved');
        }
    } catch (e) {
        showSettingsFeedback(`✕ Save failed: ${e.message}`, true);
    }
}

function resetOrchestratorPrompt() {
    if (orchSystemPrompt) orchSystemPrompt.value = DEFAULT_ORCHESTRATOR_PROMPT;
}

async function applyConversationConfig(sess) {
    if (!sess || !currentConfig) return;
    let changed = false;

    if (typeof sess.orchestrator_mode === 'boolean') {
        if (!currentConfig.orchestrator) currentConfig.orchestrator = {};
        if (Boolean(currentConfig.orchestrator.enabled) !== sess.orchestrator_mode) {
            currentConfig.orchestrator.enabled = sess.orchestrator_mode;
            changed = true;
        }
    }

    if (sess.server_id && sess.server_id !== currentConfig.active_server_id) {
        currentConfig.active_server_id = sess.server_id;
        if (Array.isArray(currentConfig.servers)) {
            currentConfig.servers.forEach(s => { s.is_default = (s.id === sess.server_id); });
        }
        renderServerPopover();
        renderConfiguredServers();
        changed = true;
    }

    if (sess.provider_id && Array.isArray(currentConfig.custom_providers)) {
        const targetProv = currentConfig.custom_providers.find(p => p.id === sess.provider_id);
        if (targetProv && currentConfig.provider !== targetProv.id) {
            currentConfig.custom_providers.forEach(p => { p.is_active = (p.id === targetProv.id); });
            currentConfig.provider = targetProv.id;
            if (targetProv.api_base) currentConfig.api_base = targetProv.api_base;
            if (targetProv.api_key) currentConfig.api_key = targetProv.api_key;
            if (cfgApiBase) cfgApiBase.value = currentConfig.api_base || '';
            if (cfgApiKey) cfgApiKey.value = currentConfig.api_key || '';
            if (labelActiveProviderName) labelActiveProviderName.textContent = targetProv.name;
            changed = true;
        }
    }

    if (sess.model && sess.model !== 'mock' && sess.model !== currentConfig.model) {
        currentConfig.model = sess.model;
        if (cfgModel) cfgModel.value = sess.model;
        changed = true;
    }

    if (sess.reasoning_effort && sess.reasoning_effort !== currentConfig.reasoning_effort) {
        currentConfig.reasoning_effort = sess.reasoning_effort;
        changed = true;
    }

    updateModelPillDisplay();
    syncOrchestratorControls();
    renderFilteredModels(true);
    renderConfiguredProviders();
    if (changed) {
        persistCurrentConfigInBackground();
    }
}

function syncActiveConversationConfig() {
    if (!currentSessionId || !currentConfig) return;
    const body = {
        model: currentConfig.model || '',
        provider_id: currentConfig.provider || '',
        reasoning_effort: currentConfig.reasoning_effort || '',
        server_id: currentConfig.active_server_id || 'local',
        orchestrator_mode: Boolean(currentConfig.orchestrator && currentConfig.orchestrator.enabled)
    };
    fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body)
    }).then(r => {
        if (r.ok) {
            const localSess = allSessions.find(s => s.id === currentSessionId);
            if (localSess) Object.assign(localSess, body);
        }
    }).catch(() => {});
}

async function applyProjectDefaults(project) {
    if (!project || !project.defaults || !currentConfig) return;
    const d = project.defaults;
    let changed = false;

    if (typeof d.orchestrator_mode === 'boolean') {
        if (!currentConfig.orchestrator) currentConfig.orchestrator = {};
        if (Boolean(currentConfig.orchestrator.enabled) !== d.orchestrator_mode) {
            currentConfig.orchestrator.enabled = d.orchestrator_mode;
            changed = true;
        }
    }
    if (d.server_id && d.server_id !== currentConfig.active_server_id) {
        currentConfig.active_server_id = d.server_id;
        if (Array.isArray(currentConfig.servers)) {
            currentConfig.servers.forEach(s => { s.is_default = (s.id === d.server_id); });
        }
        renderServerPopover();
        renderConfiguredServers();
        changed = true;
    }
    if (d.provider_id && Array.isArray(currentConfig.custom_providers)) {
        const targetProv = currentConfig.custom_providers.find(p => p.id === d.provider_id);
        if (targetProv && currentConfig.provider !== targetProv.id) {
            currentConfig.custom_providers.forEach(p => { p.is_active = (p.id === targetProv.id); });
            currentConfig.provider = targetProv.id;
            if (targetProv.api_base) currentConfig.api_base = targetProv.api_base;
            if (targetProv.api_key) currentConfig.api_key = targetProv.api_key;
            if (labelActiveProviderName) labelActiveProviderName.textContent = targetProv.name;
            changed = true;
        }
    }
    if (d.model && d.model !== currentConfig.model) {
        currentConfig.model = d.model;
        if (cfgModel) cfgModel.value = d.model;
        changed = true;
    }
    if (d.reasoning_effort && d.reasoning_effort !== currentConfig.reasoning_effort) {
        currentConfig.reasoning_effort = d.reasoning_effort;
        changed = true;
    }

    if (changed) {
        updateModelPillDisplay();
        syncOrchestratorControls();
        renderFilteredModels(true);
        renderConfiguredProviders();
        persistCurrentConfigInBackground();
    }
}

function updateProjDefPickerLabels() {
    if (!draftProjSettings) return;
    const d = draftProjSettings.defaults;
    if (labelProjDefModel) {
        labelProjDefModel.textContent = d.model ? d.model.split('/').pop() : 'Use Global Default';
    }
    if (labelProjDefEffort) {
        labelProjDefEffort.textContent = d.model && d.reasoning_effort ? formatReasoningLevelLabel(d.reasoning_effort) : '';
    }
    if (labelProjDefProviderHint) {
        labelProjDefProviderHint.textContent = d.model ? getProviderNameLabel(d.provider_id) : 'Inherits active workspace model';
    }
}

function openProjectSettingsModal(projectId) {
    const project = allProjects.find(p => p.id === projectId);
    if (!project || !projectSettingsModal) return;
    closeAllPopovers();
    const def = project.defaults || {};
    draftProjSettings = {
        id: project.id,
        name: project.name,
        icon: project.icon || '📁',
        defaults: {
            provider_id: def.provider_id || '',
            model: def.model || '',
            reasoning_effort: def.reasoning_effort || '',
            server_id: def.server_id || '',
            orchestrator_mode: typeof def.orchestrator_mode === 'boolean' ? def.orchestrator_mode : null
        }
    };

    if (projSettingsIcon) projSettingsIcon.textContent = project.icon || '📁';
    if (projSettingsTitle) projSettingsTitle.textContent = `${project.name} — Default Settings`;
    if (projDefName) projDefName.value = project.name || '';
    if (projDefIcon) projDefIcon.value = project.icon || '📁';
    if (projDefPath) projDefPath.value = project.path || '';
    if (projDefServer) {
        const servers = (currentConfig && currentConfig.servers) || [];
        projDefServer.innerHTML = '<option value="">Use Active Server</option>' +
            servers.map(s => `<option value="${escapeHtml(s.id)}">${escapeHtml(s.name)} (${escapeHtml(s.id)})</option>`).join('');
        projDefServer.value = draftProjSettings.defaults.server_id || '';
    }
    if (projDefOrchestrator) {
        projDefOrchestrator.value = draftProjSettings.defaults.orchestrator_mode === true
            ? 'enabled'
            : (draftProjSettings.defaults.orchestrator_mode === false ? 'disabled' : 'inherit');
    }
    updateProjDefPickerLabels();
    projectSettingsModal.classList.remove('hidden');
}

function closeProjectSettingsModal() {
    if (projectSettingsModal) projectSettingsModal.classList.add('hidden');
    draftProjSettings = null;
}

async function saveProjectDefaults() {
    if (!draftProjSettings) return;
    const orchSel = projDefOrchestrator ? projDefOrchestrator.value : 'inherit';
    draftProjSettings.defaults.server_id = projDefServer ? projDefServer.value : '';
    draftProjSettings.defaults.orchestrator_mode = orchSel === 'enabled' ? true : (orchSel === 'disabled' ? false : null);

    const payload = {
        name: projDefName ? projDefName.value.trim() : draftProjSettings.name,
        icon: projDefIcon ? projDefIcon.value.trim() : draftProjSettings.icon,
        path: projDefPath ? projDefPath.value.trim() : undefined,
        defaults: draftProjSettings.defaults
    };
    try {
        const res = await fetch(`/api/projects/${encodeURIComponent(draftProjSettings.id)}/defaults`, {
            method: 'PATCH',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({}));
            throw new Error(err.error || `HTTP ${res.status}`);
        }
        const savedId = draftProjSettings.id;
        closeProjectSettingsModal();
        await fetchProjects();
        if (savedId === activeProjectId && activeComposerScope === 'project') {
            await applyProjectDefaults(allProjects.find(p => p.id === savedId));
        }
        showToast('Project default settings saved.', 'success');
    } catch (e) {
        showToast('Failed to save project settings: ' + e.message, 'error');
    }
}

function renderServerPopover() {
    const servers = (currentConfig && currentConfig.servers) || allServers || [];
    const activeId = (currentConfig && currentConfig.active_server_id) || 'local';
    const activeSrv = servers.find(s => s.id === activeId) || servers[0];
    if (labelActiveServer) labelActiveServer.textContent = activeSrv ? activeSrv.name : 'Local Server';
    if (serverStatusDot && activeSrv) {
        const health = serverHealthMap.get(activeSrv.id);
        serverStatusDot.className = 'dot-live' + (health === false ? ' offline' : '');
    }
    if (!serverPopoverList) return;
    serverPopoverList.innerHTML = '';
    servers.forEach(s => {
        const isCurrent = s.id === activeId;
        const health = serverHealthMap.get(s.id);
        const statusStr = health === true ? 'online' : (health === false ? 'offline' : (s.url ? 'remote' : 'local'));
        const item = document.createElement('button');
        item.type = 'button';
        item.className = `popover-item${isCurrent ? ' active' : ''}`;
        item.innerHTML = `
            <span class="popover-item-title">${escapeHtml(s.name)} <span style="opacity:0.65;font-size:0.72rem;">(${escapeHtml(s.id)})</span></span>
            <span class="popover-item-sub">${escapeHtml(s.url || 'Built-in Local Execution')} · ${escapeHtml(statusStr)}</span>
        `;
        item.addEventListener('click', () => selectActiveServer(s.id));
        serverPopoverList.appendChild(item);
    });
}

async function selectActiveServer(serverId) {
    try {
        const res = await fetch(`/api/servers/${encodeURIComponent(serverId)}/active`, {
            method: 'POST'
        });
        if (res.ok && currentConfig) {
            currentConfig.active_server_id = serverId;
            if (Array.isArray(currentConfig.servers)) {
                currentConfig.servers.forEach(s => { s.is_default = (s.id === serverId); });
            }
            renderServerPopover();
            renderConfiguredServers();
            syncActiveConversationConfig();
            closeAllPopovers();
            showToast(`Primary execution server set to '${serverId}'.`, 'success');
        }
    } catch (e) {
        showToast('Failed to switch server: ' + e.message, 'error');
    }
}

async function probeServerNode(srv) {
    if (!srv) return;
    if (!srv.url || srv.id === 'local' || srv.url.startsWith('local://')) {
        serverHealthMap.set(srv.id, true);
        renderServerPopover();
        renderConfiguredServers();
        return;
    }
    try {
        const res = await fetch(`/api/servers/${encodeURIComponent(srv.id)}/health`);
        if (res.ok) {
            const data = await res.json();
            serverHealthMap.set(srv.id, Boolean(data.online ?? data.ok));
        } else {
            serverHealthMap.set(srv.id, false);
        }
    } catch (_) {
        serverHealthMap.set(srv.id, false);
    }
    renderServerPopover();
    renderConfiguredServers();
}

async function probeAllServers() {
    const servers = (currentConfig && currentConfig.servers) || [];
    await Promise.all(servers.map(s => probeServerNode(s)));
}

function renderConfiguredServers() {
    if (!configuredServersContainer) return;
    const servers = (currentConfig && currentConfig.servers) || [];
    const activeId = (currentConfig && currentConfig.active_server_id) || 'local';
    configuredServersContainer.innerHTML = '';

    servers.forEach(s => {
        const isActive = s.id === activeId;
        const health = serverHealthMap.get(s.id);
        const healthLabel = health === true ? 'Online' : (health === false ? 'Unreachable' : (s.url ? 'Remote' : 'Local'));
        const card = document.createElement('div');
        card.className = `provider-card-item ${isActive ? 'active' : ''}`;
        card.innerHTML = `
            <div class="provider-card-meta">
                <div class="provider-card-title-row">
                    <span class="provider-card-name">${escapeHtml(s.name)}</span>
                    ${isActive ? '<span class="badge-active-provider">Primary</span>' : ''}
                    <span class="model-provider-badge${health === false ? ' missing-key' : ''}">${escapeHtml(healthLabel)}</span>
                </div>
                <span class="provider-card-url">${escapeHtml(s.url || 'Built-in Local Tool Execution')}</span>
                <span class="provider-card-model">ID: ${escapeHtml(s.id)}</span>
            </div>
            <div class="provider-card-actions">
                ${!isActive ? `<button type="button" class="btn-card-action primary" data-srv-use="${escapeHtml(s.id)}">Use Primary</button>` : ''}
                <button type="button" class="btn-card-action" data-srv-probe="${escapeHtml(s.id)}">Health</button>
                <button type="button" class="btn-card-action" data-srv-edit="${escapeHtml(s.id)}">Edit</button>
                ${s.id !== 'local' ? `<button type="button" class="btn-card-action danger" data-srv-del="${escapeHtml(s.id)}">Delete</button>` : ''}
            </div>
        `;
        card.querySelector('[data-srv-use]')?.addEventListener('click', () => selectActiveServer(s.id));
        card.querySelector('[data-srv-probe]')?.addEventListener('click', () => probeServerNode(s));
        card.querySelector('[data-srv-edit]')?.addEventListener('click', () => editServerNode(s.id));
        card.querySelector('[data-srv-del]')?.addEventListener('click', () => deleteServerNode(s.id));
        configuredServersContainer.appendChild(card);
    });
}

function showAddServerForm() {
    if (!serverEditorCard) return;
    serverEditorCard.classList.remove('hidden');
    if (editServerName) editServerName.value = '';
    if (editServerId) editServerId.value = '';
    if (editServerUrl) editServerUrl.value = '';
    if (editServerToken) editServerToken.value = '';
    if (editServerDefault) editServerDefault.checked = false;
    const t = document.getElementById('server-editor-title');
    if (t) t.textContent = 'Add Execution Server';
    if (editServerName) editServerName.focus();
}

function hideAddServerForm() {
    if (serverEditorCard) serverEditorCard.classList.add('hidden');
}

function editServerNode(serverId) {
    const s = (currentConfig && currentConfig.servers || []).find(x => x.id === serverId);
    if (!s) return;
    showAddServerForm();
    const t = document.getElementById('server-editor-title');
    if (t) t.textContent = `Edit Server: ${s.name}`;
    if (editServerName) editServerName.value = s.name;
    if (editServerId) editServerId.value = s.id;
    if (editServerUrl) editServerUrl.value = s.url;
    if (editServerToken) editServerToken.value = s.api_token || '';
    if (editServerDefault) editServerDefault.checked = Boolean(s.is_default);
}

async function saveServerNode() {
    const name = editServerName ? editServerName.value.trim() : '';
    const id = (editServerId ? editServerId.value.trim() : '') || name.toLowerCase().replace(/[^a-z0-9_-]/g, '-');
    const url = editServerUrl ? editServerUrl.value.trim() : '';
    const token = editServerToken ? editServerToken.value.trim() : '';
    const isDefault = editServerDefault ? !!editServerDefault.checked : false;

    if (!name || !id) {
        showToast('Server Name and ID are required.', 'warning');
        return;
    }
    if (id !== 'local' && !url) {
        showToast('Remote Server URL is required (e.g. http://127.0.0.1:8082).', 'warning');
        return;
    }
    try {
        const res = await fetch('/api/servers', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                id,
                name,
                url,
                api_token: token,
                enabled: true,
                is_default: isDefault
            })
        });
        if (!res.ok) {
            const errMsg = await res.text().catch(() => `HTTP ${res.status}`);
            throw new Error(errMsg || `HTTP ${res.status}`);
        }
        const listRes = await fetch('/api/servers');
        if (listRes.ok && currentConfig) {
            const srvData = await listRes.json();
            currentConfig.servers = Array.isArray(srvData.servers) ? srvData.servers : (currentConfig.servers || []);
            if (srvData.active_server_id) currentConfig.active_server_id = srvData.active_server_id;
        }
        hideAddServerForm();
        renderServerPopover();
        renderConfiguredServers();
        syncOrchestratorControls();
        probeAllServers();
        showSettingsFeedback(`✓ Saved server '${name}'`);
    } catch (e) {
        showSettingsFeedback(`✕ Failed to save server: ${e.message}`, true);
    }
}

async function deleteServerNode(serverId) {
    const ok = await uiConfirm(`Remove execution server '${serverId}'?`, {
        title: 'Delete Server',
        confirmText: 'Remove',
        danger: true
    });
    if (!ok) return;
    try {
        const res = await fetch(`/api/servers/${encodeURIComponent(serverId)}`, { method: 'DELETE' });
        if (res.ok) {
            const listRes = await fetch('/api/servers');
            if (listRes.ok && currentConfig) {
                const srvData = await listRes.json();
                currentConfig.servers = Array.isArray(srvData.servers) ? srvData.servers : (currentConfig.servers || []);
                currentConfig.active_server_id = srvData.active_server_id || 'local';
            }
            renderServerPopover();
            renderConfiguredServers();
            syncOrchestratorControls();
            showToast(`Removed server '${serverId}'.`, 'info');
        }
    } catch (e) {
        showToast('Failed to delete server: ' + e.message, 'error');
    }
}

async function saveConfig() {
    if (!currentConfig) return;
    if (cfgApiBase) currentConfig.api_base = cfgApiBase.value.trim();
    if (cfgApiKey) currentConfig.api_key = cfgApiKey.value.trim();
    if (cfgModel) currentConfig.model = cfgModel.value.trim();
    if (cfgReasoningEffort) currentConfig.reasoning_effort = cfgReasoningEffort.value;
    currentConfig.max_iterations = 0;
    if (cfgMaxOut) currentConfig.max_tool_output_bytes = parseInt(cfgMaxOut.value, 10) || 16384;
    if (cfgSandbox) currentConfig.sandbox_root = cfgSandbox.value.trim();
    currentConfig.display_verbosity = activeDisplayVerbosity;
    currentConfig.send_mode = activeSendMode;
    if (cfgVoiceActivityDetection) currentConfig.voice_activity_detection = !!cfgVoiceActivityDetection.checked;
    if (cfgAutoUpdateCheck) currentConfig.auto_update_check = !!cfgAutoUpdateCheck.checked;
    if (cfgAutoUpdateApply) currentConfig.auto_update_apply = !!cfgAutoUpdateApply.checked;
    if (cfgAutoSkillRouter) currentConfig.auto_skill_router = !!cfgAutoSkillRouter.checked;
    if (cfgSkillRouterConfidence) {
        const v = parseFloat(cfgSkillRouterConfidence.value);
        currentConfig.skill_router_min_confidence = Number.isFinite(v)
            ? Math.min(0.95, Math.max(0.05, v))
            : 0.35;
    }
    if (cfgSkillRouterMax) {
        const v = parseInt(cfgSkillRouterMax.value, 10);
        currentConfig.skill_router_max_skills = Number.isFinite(v)
            ? Math.min(4, Math.max(0, v))
            : 2;
    }

    const prevMem = currentConfig.memory || {};
    const autoDetectCtx = cfgAutoDetectContext ? !!cfgAutoDetectContext.checked : (prevMem.auto_detect_context !== false);
    const parsedRetries = cfgMaxToolRetries ? parseInt(cfgMaxToolRetries.value, 10) : NaN;
    currentConfig.memory = {
        auto_detect_context: autoDetectCtx,
        max_context_tokens: cfgMaxContextTokens ? (parseInt(cfgMaxContextTokens.value, 10) || 128000) : (prevMem.max_context_tokens ?? 128000),
        auto_compact_threshold_pct: cfgAutoCompactPct ? (parseInt(cfgAutoCompactPct.value, 10) || 78) : (prevMem.auto_compact_threshold_pct ?? 78),
        preserve_recent_messages: cfgPreserveRecent ? (parseInt(cfgPreserveRecent.value, 10) || 8) : (prevMem.preserve_recent_messages ?? 8),
        prune_tool_outputs_over_bytes: cfgPruneToolBytes ? (parseInt(cfgPruneToolBytes.value, 10) || 2048) : (prevMem.prune_tool_outputs_over_bytes ?? 2048),
        summarize_compacted_turns: cfgSummarizeCompactedTurns ? !!cfgSummarizeCompactedTurns.checked : (prevMem.summarize_compacted_turns !== false),
        enable_working_memory: cfgEnableWorkingMemory ? !!cfgEnableWorkingMemory.checked : (prevMem.enable_working_memory !== false),
        max_tool_retries: Number.isNaN(parsedRetries) ? (prevMem.max_tool_retries ?? 3) : Math.max(0, Math.min(10, parsedRetries))
    };

    try {
        const result = await saveCurrentConfig();
        if (result.ok) {
            if (labelActiveModel && currentConfig.model) {
                labelActiveModel.textContent = currentConfig.model.split('/').pop();
            }
            setReasoningEffort(currentConfig.reasoning_effort, false, false);
            refreshReasoningControlsForModel(currentConfig.model || '', false);
            syncModelContextWindowIfAuto(currentConfig.model || '');
            renderFilteredModels();
            showSettingsFeedback('✓ Memory & runtime settings saved');
        }
    } catch (e) {
        showSettingsFeedback(`✕ Failed to save config: ${e.message}`, true);
    }
}

function syncPermissionControls() {
    if (!currentConfig || !currentConfig.permissions) return;
    const p = currentConfig.permissions;
    setSegActive('bash', p.bash_policy);
    setSegActive('file_write', p.file_write_policy);
    setSegActive('file_read', p.file_read_policy);
    setSegActive('web', p.web_policy);

    const permGlobalMode = document.getElementById('perm-global-mode');
    if (permGlobalMode) {
        permGlobalMode.querySelectorAll('.seg-btn').forEach(btn => {
            btn.classList.toggle('active', btn.getAttribute('data-val') === (p.mode || 'safe'));
        });
    }
}

function setSegActive(policyName, val) {
    const ctrl = document.querySelector(`.segmented-control[data-policy="${policyName}"]`);
    if (!ctrl) return;
    ctrl.querySelectorAll('.seg-btn').forEach(btn => {
        btn.classList.toggle('active', btn.getAttribute('data-val') === val);
    });
}

function getSegActive(policyName) {
    const ctrl = document.querySelector(`.segmented-control[data-policy="${policyName}"]`);
    if (!ctrl) return 'allow';
    const active = ctrl.querySelector('.seg-btn.active');
    return active ? active.getAttribute('data-val') : 'allow';
}

async function savePermissions() {
    if (!currentConfig) return;
    const globalModeBtn = document.querySelector('#perm-global-mode .seg-btn.active');
    const globalMode = globalModeBtn ? globalModeBtn.getAttribute('data-val') : 'safe';

    const requested = {
        mode: globalMode,
        bash_policy: getSegActive('bash'),
        file_write_policy: getSegActive('file_write'),
        file_read_policy: getSegActive('file_read'),
        web_policy: getSegActive('web')
    };

    // `POST /api/config` ignores the permission policy on purpose (a stale
    // snapshot must not be able to turn the whole harness into auto-approve), so
    // this save goes through the dedicated endpoint and adopts what the server
    // echoes back. Before, this handler reported success on a no-op.
    try {
        const res = await fetch('/api/security/permissions', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(requested)
        });
        if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
        const saved = await res.json();
        const perms = saved.permissions || requested;
        if (currentConfig.permissions) Object.assign(currentConfig.permissions, perms);
        setPermissionMode(perms.mode, false);
        if (typeof syncPermissionControls === 'function') syncPermissionControls();
        showSettingsFeedback(`Tool permissions saved (${describePermissionMode(perms.mode)}, bash: ${perms.bash_policy}).`);
        if (perms.mode !== globalMode) {
            showSettingsFeedback(`Note: the server applied ${describePermissionMode(perms.mode)}, not ${describePermissionMode(globalMode)}.`, true);
        }
    } catch (e) {
        showSettingsFeedback(`Failed to save permissions: ${e.message}`, true);
    }
}

function describePermissionMode(mode) {
    return { 'safe': 'Safe (Ask)', 'auto': 'Auto', 'autonomous': 'Autonomous', 'yolo': 'Yolo', 'read_only': 'Read Only' }[mode] || mode;
}

let cachedToolsList = [];
let toolsActiveFilter = 'all';
let toolsSearchTerm = '';
const expandedToolCards = new Set();
let toolsModalControlsBound = false;

function openToolsModal() {
    if (!toolsModal) return;
    toolsModal.classList.remove('hidden');
    loadTools();
}

function initToolsModalControls() {
    if (toolsModalControlsBound) return;
    toolsModalControlsBound = true;

    const btnToggleAdd = document.getElementById('btn-toggle-add-tool');
    const btnCancelCt = document.getElementById('btn-cancel-custom-tool');
    const btnAddField = document.getElementById('btn-add-ct-field');
    const btnSaveCt = document.getElementById('btn-save-custom-tool');
    const btnResetCt = document.getElementById('btn-reset-custom-tool');
    const btnRefresh = document.getElementById('btn-refresh-tools');
    const searchInput = document.getElementById('tools-search-input');
    const runnerCtrl = document.getElementById('ct-runner-type-ctrl');

    if (btnToggleAdd) {
        btnToggleAdd.addEventListener('click', () => {
            const builder = document.getElementById('custom-tool-builder-card');
            if (!builder) return;
            const willOpen = builder.classList.contains('hidden');
            toggleCustomToolBuilder(willOpen);
        });
    }
    if (btnCancelCt) {
        btnCancelCt.addEventListener('click', () => toggleCustomToolBuilder(false));
    }
    if (btnAddField) {
        btnAddField.addEventListener('click', () => addCustomToolFieldRow());
    }
    if (btnSaveCt) {
        btnSaveCt.addEventListener('click', saveCustomToolFromBuilder);
    }
    if (btnResetCt) {
        btnResetCt.addEventListener('click', resetCustomToolBuilder);
    }
    if (btnRefresh) {
        btnRefresh.addEventListener('click', () => loadTools());
    }
    if (searchInput) {
        searchInput.addEventListener('input', (e) => {
            toolsSearchTerm = (e.target.value || '').trim().toLowerCase();
            renderToolsList();
        });
    }
    if (runnerCtrl) {
        runnerCtrl.querySelectorAll('.seg-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                const mode = btn.getAttribute('data-runner-type') || 'shell';
                switchCustomToolRunnerMode(mode);
            });
        });
    }

    ensureCustomToolFieldsPlaceholder();
}

function toggleCustomToolBuilder(show) {
    const builder = document.getElementById('custom-tool-builder-card');
    const btnToggle = document.getElementById('btn-toggle-add-tool');
    if (!builder) return;
    if (show) {
        builder.classList.remove('hidden');
        if (btnToggle) btnToggle.textContent = '− Hide Builder';
        const list = document.getElementById('ct-fields-list');
        if (list && list.querySelectorAll('.ct-field-row').length === 0) {
            addCustomToolFieldRow();
        }
        const nameEl = document.getElementById('ct-name');
        if (nameEl) nameEl.focus();
    } else {
        builder.classList.add('hidden');
        if (btnToggle) btnToggle.textContent = '+ Add Extra Tool';
    }
}

function switchCustomToolRunnerMode(mode) {
    const shellPanel = document.getElementById('ct-runner-shell');
    const httpPanel = document.getElementById('ct-runner-http');
    const runnerCtrl = document.getElementById('ct-runner-type-ctrl');
    if (runnerCtrl) {
        runnerCtrl.querySelectorAll('.seg-btn').forEach(b => {
            b.classList.toggle('active', b.getAttribute('data-runner-type') === mode);
        });
    }
    if (shellPanel) shellPanel.classList.toggle('hidden', mode !== 'shell');
    if (httpPanel) httpPanel.classList.toggle('hidden', mode !== 'http');
}

function getActiveCustomToolRunnerMode() {
    const activeBtn = document.querySelector('#ct-runner-type-ctrl .seg-btn.active');
    return activeBtn ? (activeBtn.getAttribute('data-runner-type') || 'shell') : 'shell';
}

function ensureCustomToolFieldsPlaceholder() {
    const list = document.getElementById('ct-fields-list');
    if (!list) return;
    const rows = list.querySelectorAll('.ct-field-row');
    const existingEmpty = list.querySelector('.ct-fields-empty');
    if (rows.length === 0 && !existingEmpty) {
        const empty = document.createElement('div');
        empty.className = 'ct-fields-empty';
        empty.textContent = 'No input fields yet. Click "+ Add Field" to define typed parameters and validation hints.';
        list.appendChild(empty);
    } else if (rows.length > 0 && existingEmpty) {
        existingEmpty.remove();
    }
}

function addCustomToolFieldRow(preset = null) {
    const list = document.getElementById('ct-fields-list');
    if (!list) return;

    const row = document.createElement('div');
    row.className = 'ct-field-row';
    const f = preset || {};
    const fType = f.field_type || 'string';
    const enumStr = Array.isArray(f.enum_values) ? f.enum_values.join(', ') : '';
    const defaultStr = f.default !== undefined && f.default !== null
        ? (typeof f.default === 'string' ? f.default : JSON.stringify(f.default))
        : '';

    row.innerHTML = `
        <input type="text" class="form-input mono-input ct-f-name" placeholder="field_name" value="${escapeHtml(f.name || '')}">
        <div class="ui-select compact" data-value="${escapeHtml(fType)}">
            <input type="hidden" class="ct-f-type" value="${escapeHtml(fType)}">
            <button type="button" class="ui-select-trigger" aria-haspopup="listbox" aria-expanded="false">
                <span class="ui-select-label">${escapeHtml(fType)}</span>
                <span class="ui-select-chevron">▾</span>
            </button>
            <div class="ui-select-menu hidden" role="listbox">
                <button type="button" class="ui-select-option${fType === 'string' ? ' active' : ''}" data-value="string">string</button>
                <button type="button" class="ui-select-option${fType === 'integer' ? ' active' : ''}" data-value="integer">integer</button>
                <button type="button" class="ui-select-option${fType === 'number' ? ' active' : ''}" data-value="number">number</button>
                <button type="button" class="ui-select-option${fType === 'boolean' ? ' active' : ''}" data-value="boolean">boolean</button>
                <button type="button" class="ui-select-option${fType === 'array' ? ' active' : ''}" data-value="array">array</button>
                <button type="button" class="ui-select-option${fType === 'object' ? ' active' : ''}" data-value="object">object</button>
            </div>
        </div>
        <label class="ct-check-pill ui-switch-row" title="Field is required">
            <span class="ui-switch compact">
                <input type="checkbox" class="ct-f-req"${f.required !== false ? ' checked' : ''}>
                <span class="ui-switch-track"><span class="ui-switch-thumb"></span></span>
            </span>
            <span>Req</span>
        </label>
        <label class="ct-check-pill ui-switch-row" title="String must be non-empty">
            <span class="ui-switch compact">
                <input type="checkbox" class="ct-f-nonempty"${f.non_empty ? ' checked' : ''}>
                <span class="ui-switch-track"><span class="ui-switch-thumb"></span></span>
            </span>
            <span>NonEmpty</span>
        </label>
        <input type="text" class="form-input ct-f-enum" placeholder="enum: a, b" value="${escapeHtml(enumStr)}">
        <input type="text" class="form-input ct-f-default" placeholder="default" value="${escapeHtml(defaultStr)}">
        <input type="text" class="form-input ct-f-desc" placeholder="description" value="${escapeHtml(f.description || '')}">
        <input type="text" class="form-input ct-f-hint" placeholder="error hint" value="${escapeHtml(f.error_hint || '')}">
        <button type="button" class="ct-remove-field-btn" title="Remove field">✕</button>
    `;

    const fieldTypeSelect = row.querySelector('.ui-select');
    if (fieldTypeSelect) bindCustomSelect(fieldTypeSelect);

    const rmBtn = row.querySelector('.ct-remove-field-btn');
    if (rmBtn) {
        rmBtn.addEventListener('click', () => {
            row.remove();
            ensureCustomToolFieldsPlaceholder();
        });
    }

    list.appendChild(row);
    ensureCustomToolFieldsPlaceholder();
}

function parseFieldDefaultValue(raw, fieldType) {
    const trimmed = (raw || '').trim();
    if (!trimmed) return undefined;
    if (fieldType === 'integer') {
        const n = parseInt(trimmed, 10);
        return Number.isNaN(n) ? trimmed : n;
    }
    if (fieldType === 'number') {
        const n = Number(trimmed);
        return Number.isNaN(n) ? trimmed : n;
    }
    if (fieldType === 'boolean') {
        if (trimmed.toLowerCase() === 'true') return true;
        if (trimmed.toLowerCase() === 'false') return false;
        return trimmed;
    }
    if (fieldType === 'array' || fieldType === 'object') {
        try {
            return JSON.parse(trimmed);
        } catch (_) {
            return trimmed;
        }
    }
    return trimmed;
}

function collectCustomToolFields() {
    const list = document.getElementById('ct-fields-list');
    if (!list) return [];
    const fields = [];
    list.querySelectorAll('.ct-field-row').forEach(row => {
        const name = (row.querySelector('.ct-f-name')?.value || '').trim();
        if (!name) return;
        const fieldType = row.querySelector('.ct-f-type')?.value || 'string';
        const required = Boolean(row.querySelector('.ct-f-req')?.checked);
        const nonEmpty = Boolean(row.querySelector('.ct-f-nonempty')?.checked);
        const enumRaw = (row.querySelector('.ct-f-enum')?.value || '').trim();
        const defaultRaw = (row.querySelector('.ct-f-default')?.value || '').trim();
        const description = (row.querySelector('.ct-f-desc')?.value || '').trim();
        const errorHint = (row.querySelector('.ct-f-hint')?.value || '').trim();

        const spec = {
            name,
            field_type: fieldType,
            description,
            required,
            non_empty: nonEmpty,
            error_hint: errorHint
        };
        if (enumRaw) {
            const enumVals = enumRaw.split(',').map(s => s.trim()).filter(Boolean);
            if (enumVals.length > 0) spec.enum_values = enumVals;
        }
        const parsedDefault = parseFieldDefaultValue(defaultRaw, fieldType);
        if (parsedDefault !== undefined) spec.default = parsedDefault;
        fields.push(spec);
    });
    return fields;
}

function showCustomToolBuilderStatus(message, isError = false) {
    const el = document.getElementById('ct-status-msg');
    if (!el) return;
    el.textContent = message;
    el.className = `provider-test-status ${isError ? 'error' : 'success'}`;
    el.classList.remove('hidden');
}

function showToolsActionFeedback(message, isError = false) {
    const el = document.getElementById('tools-action-feedback');
    if (!el) return;
    el.textContent = message;
    el.className = `provider-test-status ${isError ? 'error' : 'success'}`;
    el.classList.remove('hidden');
    clearTimeout(el._hideTimer);
    el._hideTimer = setTimeout(() => el.classList.add('hidden'), 5000);
}

function resetCustomToolBuilder() {
    const ids = ['ct-name', 'ct-description', 'ct-out-desc', 'ct-failure-hint', 'ct-shell-cmd', 'ct-http-url', 'ct-http-body'];
    ids.forEach(id => {
        const el = document.getElementById(id);
        if (el) el.value = '';
    });
    const cat = document.getElementById('ct-category');
    if (cat) cat.value = 'custom';
    const perm = document.getElementById('ct-permission');
    if (perm) setCustomSelectValue(perm, 'shell_or_network');
    const timeout = document.getElementById('ct-timeout');
    if (timeout) timeout.value = '30';
    const outFmt = document.getElementById('ct-out-format');
    if (outFmt) setCustomSelectValue(outFmt, 'text');
    const httpMethod = document.getElementById('ct-http-method');
    if (httpMethod) setCustomSelectValue(httpMethod, 'GET');
    const list = document.getElementById('ct-fields-list');
    if (list) list.innerHTML = '';
    ensureCustomToolFieldsPlaceholder();
    switchCustomToolRunnerMode('shell');
    const statusEl = document.getElementById('ct-status-msg');
    if (statusEl) statusEl.classList.add('hidden');
}

async function saveCustomToolFromBuilder() {
    const name = (document.getElementById('ct-name')?.value || '').trim();
    const category = (document.getElementById('ct-category')?.value || '').trim() || 'custom';
    const permissionLevel = document.getElementById('ct-permission')?.value || 'shell_or_network';
    const timeoutRaw = parseInt(document.getElementById('ct-timeout')?.value || '30', 10);
    const timeoutSecs = Number.isFinite(timeoutRaw) && timeoutRaw > 0 ? timeoutRaw : 30;
    const description = (document.getElementById('ct-description')?.value || '').trim();
    const outFormat = document.getElementById('ct-out-format')?.value || 'text';
    const outDesc = (document.getElementById('ct-out-desc')?.value || '').trim() || 'Tool execution output';
    const failureHint = (document.getElementById('ct-failure-hint')?.value || '').trim();
    const runnerMode = getActiveCustomToolRunnerMode();

    if (!/^[A-Za-z][A-Za-z0-9_]{0,63}$/.test(name)) {
        showCustomToolBuilderStatus('Tool name must start with a letter and use only letters, digits, and underscores (max 64 chars).', true);
        return;
    }
    if (!description) {
        showCustomToolBuilderStatus('Description is required so the agent knows when to invoke this tool.', true);
        return;
    }

    let runner;
    if (runnerMode === 'http') {
        const method = document.getElementById('ct-http-method')?.value || 'GET';
        const urlTemplate = (document.getElementById('ct-http-url')?.value || '').trim();
        const bodyTemplate = (document.getElementById('ct-http-body')?.value || '').trim();
        if (!urlTemplate) {
            showCustomToolBuilderStatus('HTTP runner requires a URL template.', true);
            return;
        }
        runner = {
            type: 'http',
            method,
            url_template: urlTemplate,
            headers: {}
        };
        if (bodyTemplate) runner.body_template = bodyTemplate;
    } else {
        const cmdTemplate = (document.getElementById('ct-shell-cmd')?.value || '').trim();
        if (!cmdTemplate) {
            showCustomToolBuilderStatus('Shell runner requires a command template.', true);
            return;
        }
        runner = {
            type: 'shell',
            command_template: cmdTemplate
        };
    }

    const fields = collectCustomToolFields();
    const failureModes = failureHint
        ? [{
            code: 'execution_error',
            condition: 'Runner exits non-zero, times out, or fails validation',
            error_message: failureHint,
            recovery_hint: failureHint
        }]
        : [];

    const payload = {
        name,
        description,
        category,
        permission_level: permissionLevel,
        timeout_secs: timeoutSecs,
        fields,
        expected_output: {
            format: outFormat,
            description: outDesc,
            example: ''
        },
        failure_modes: failureModes,
        runner
    };

    const btnSave = document.getElementById('btn-save-custom-tool');
    if (btnSave) {
        btnSave.disabled = true;
        btnSave.textContent = 'Registering...';
    }

    try {
        const url = currentSessionId
            ? `/api/tools?session_id=${encodeURIComponent(currentSessionId)}`
            : '/api/tools';
        const res = await fetch(url, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });

        if (!res.ok) {
            const errText = await res.text();
            showCustomToolBuilderStatus(`Failed (${res.status}): ${errText || 'Invalid tool specification'}`, true);
            return;
        }

        const created = await res.json();
        expandedToolCards.add(created.name);
        resetCustomToolBuilder();
        toggleCustomToolBuilder(false);
        showToolsActionFeedback(`✓ Registered custom tool '${created.name}' into .harness/tools/${created.name}.json`, false);
        await loadTools();
    } catch (e) {
        showCustomToolBuilderStatus(`Request failed: ${e.message}`, true);
    } finally {
        if (btnSave) {
            btnSave.disabled = false;
            btnSave.textContent = 'Create & Register Tool';
        }
    }
}

async function deleteCustomToolFromUi(toolName) {
    const ok = await uiConfirm(`Delete custom tool '${toolName}' from .harness/tools/?`, {
        title: 'Delete Custom Tool',
        confirmText: 'Delete Tool',
        danger: true
    });
    if (!ok) return;
    try {
        const url = currentSessionId
            ? `/api/tools/${encodeURIComponent(toolName)}?session_id=${encodeURIComponent(currentSessionId)}`
            : `/api/tools/${encodeURIComponent(toolName)}`;
        const res = await fetch(url, { method: 'DELETE' });
        if (!res.ok) {
            const errText = await res.text();
            showToolsActionFeedback(`✕ Failed to delete '${toolName}': ${errText}`, true);
            return;
        }
        expandedToolCards.delete(toolName);
        showToolsActionFeedback(`✓ Deleted custom tool '${toolName}'.`, false);
        await loadTools();
    } catch (e) {
        showToolsActionFeedback(`✕ Failed to delete '${toolName}': ${e.message}`, true);
    }
}

async function validateToolContractArgsFromCard(toolName, textareaEl, resultEl) {
    if (!textareaEl || !resultEl) return;
    const raw = (textareaEl.value || '').trim() || '{}';
    let parsed;
    try {
        parsed = JSON.parse(raw);
    } catch (e) {
        resultEl.className = 'contract-validator-result invalid';
        resultEl.textContent = `✕ Invalid JSON: ${e.message}`;
        resultEl.classList.remove('hidden');
        return;
    }

    try {
        const res = await fetch(`/api/tools/${encodeURIComponent(toolName)}/validate`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(parsed)
        });
        const data = await res.json();
        if (res.ok && data.valid) {
            resultEl.className = 'contract-validator-result valid';
            resultEl.textContent = '✓ Valid arguments — passes all contract checks.';
        } else {
            resultEl.className = 'contract-validator-result invalid';
            resultEl.textContent = `✕ ${data.error || 'Contract validation failed'}`;
        }
        resultEl.classList.remove('hidden');
    } catch (e) {
        resultEl.className = 'contract-validator-result invalid';
        resultEl.textContent = `✕ Validation request error: ${e.message}`;
        resultEl.classList.remove('hidden');
    }
}

function formatPermissionLabel(level) {
    const map = {
        read_only: 'ReadOnly',
        workspace_write: 'WorkspaceWrite',
        shell_or_network: 'ShellOrNetwork',
        high_risk: 'HighRisk',
        meta: 'Meta'
    };
    return map[level] || level || 'ReadOnly';
}

function formatPolicyLabel(policy) {
    const map = { allow: 'Allow', ask: 'Ask', deny: 'Deny' };
    return map[policy] || policy || 'Ask';
}

function buildSampleArgsForContract(tool) {
    const obj = {};
    const fields = Array.isArray(tool.fields) ? tool.fields : [];
    const targetFields = fields.filter(f => f.required);
    const sampleList = targetFields.length > 0 ? targetFields : fields.slice(0, 2);

    sampleList.forEach(f => {
        if (f.default !== undefined && f.default !== null) {
            obj[f.name] = f.default;
        } else if (Array.isArray(f.enum_values) && f.enum_values.length > 0) {
            obj[f.name] = f.enum_values[0];
        } else {
            switch (f.field_type) {
                case 'integer':
                    obj[f.name] = typeof f.min === 'number' ? Math.ceil(f.min) : 1;
                    break;
                case 'number':
                    obj[f.name] = typeof f.min === 'number' ? f.min : 1.0;
                    break;
                case 'boolean':
                    obj[f.name] = true;
                    break;
                case 'array':
                    obj[f.name] = [];
                    break;
                case 'object':
                    obj[f.name] = {};
                    break;
                default:
                    obj[f.name] = '';
                    break;
            }
        }
    });
    return JSON.stringify(obj, null, 2);
}

function renderToolsSummaryStrip() {
    const strip = document.getElementById('tools-summary-strip');
    if (!strip) return;
    const total = cachedToolsList.length;
    const customCount = cachedToolsList.filter(t => t.is_custom).length;
    const builtinCount = total - customCount;
    strip.innerHTML = `
        <span class="tools-summary-chip">${total} Total</span>
        <span class="tools-summary-chip">${builtinCount} Built-in</span>
        <span class="tools-summary-chip custom">${customCount} Custom</span>
    `;
}

function renderToolsFilterPills() {
    const container = document.getElementById('tools-filter-pills');
    if (!container) return;

    const categories = Array.from(
        new Set(cachedToolsList.map(t => (t.category || 'general').trim()).filter(Boolean))
    ).sort();

    const pills = [
        { key: 'all', label: `All (${cachedToolsList.length})` },
        { key: 'builtin', label: `Built-in (${cachedToolsList.filter(t => !t.is_custom).length})` },
        { key: 'custom', label: `Custom (${cachedToolsList.filter(t => t.is_custom).length})` }
    ];
    categories.forEach(cat => {
        const count = cachedToolsList.filter(t => (t.category || 'general') === cat).length;
        pills.push({ key: `cat:${cat}`, label: `${cat} (${count})` });
    });

    if (!pills.some(p => p.key === toolsActiveFilter)) {
        toolsActiveFilter = 'all';
    }

    container.innerHTML = '';
    pills.forEach(p => {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = `tool-filter-pill${toolsActiveFilter === p.key ? ' active' : ''}`;
        btn.setAttribute('data-filter', p.key);
        btn.textContent = p.label;
        btn.addEventListener('click', () => {
            toolsActiveFilter = p.key;
            renderToolsFilterPills();
            renderToolsList();
        });
        container.appendChild(btn);
    });
}

function renderToolsList() {
    if (!toolsContainer) return;

    const filtered = cachedToolsList.filter(t => {
        if (toolsActiveFilter === 'builtin' && t.is_custom) return false;
        if (toolsActiveFilter === 'custom' && !t.is_custom) return false;
        if (toolsActiveFilter.startsWith('cat:')) {
            const targetCat = toolsActiveFilter.slice(4);
            if ((t.category || 'general') !== targetCat) return false;
        }
        if (toolsSearchTerm) {
            const haystack = [
                t.name,
                t.description,
                t.category,
                t.permission_level,
                t.effective_policy,
                ...(Array.isArray(t.fields) ? t.fields.flatMap(f => [f.name, f.description, f.error_hint]) : [])
            ].filter(Boolean).join(' ').toLowerCase();
            if (!haystack.includes(toolsSearchTerm)) return false;
        }
        return true;
    });

    toolsContainer.innerHTML = '';
    if (filtered.length === 0) {
        toolsContainer.innerHTML = '<div class="loading-state">No tools match the current filter or search query.</div>';
        return;
    }

    filtered.forEach(t => {
        const isExpanded = expandedToolCards.has(t.name) || (filtered.length === 1 && toolsSearchTerm.length > 0);
        const permKey = t.permission_level || 'read_only';
        const policyKey = t.effective_policy || 'ask';
        const fields = Array.isArray(t.fields) ? t.fields : [];
        const failureModes = Array.isArray(t.failure_modes) ? t.failure_modes : [];
        const expectedOutput = t.expected_output || {};

        const card = document.createElement('div');
        card.className = `tool-contract-card${t.is_custom ? ' is-custom' : ''}${isExpanded ? ' expanded' : ''}`;
        card.setAttribute('data-tool-name', t.name);

        const fieldPreviewHtml = fields.length > 0
            ? `<div class="tool-contract-field-pills">${fields.map(f => `<span class="tool-arg-chip${f.required ? ' required' : ''}" title="${escapeHtml(f.description || '')}">${escapeHtml(f.name)}${f.required ? '<span class="req-star">*</span>' : ''}</span>`).join('')}</div>`
            : '';

        const fieldsRowsHtml = fields.length > 0
            ? fields.map(f => {
                const constraints = [];
                if (f.default !== undefined && f.default !== null) {
                    constraints.push(`<span class="contract-constraint-chip">default: ${escapeHtml(typeof f.default === 'string' ? f.default : JSON.stringify(f.default))}</span>`);
                }
                if (Array.isArray(f.enum_values) && f.enum_values.length > 0) {
                    constraints.push(`<span class="contract-constraint-chip">enum: ${escapeHtml(f.enum_values.join(' | '))}</span>`);
                }
                if (f.non_empty) constraints.push('<span class="contract-constraint-chip">non_empty</span>');
                if (typeof f.min === 'number') constraints.push(`<span class="contract-constraint-chip">min: ${escapeHtml(String(f.min))}</span>`);
                if (typeof f.max === 'number') constraints.push(`<span class="contract-constraint-chip">max: ${escapeHtml(String(f.max))}</span>`);

                return `
                    <tr>
                        <td><span class="contract-field-name">${escapeHtml(f.name)}</span></td>
                        <td><span class="contract-field-type">${escapeHtml(f.field_type || 'string')}</span></td>
                        <td><span class="req-pill ${f.required ? 'required' : 'optional'}">${f.required ? 'Required' : 'Optional'}</span></td>
                        <td>
                            <div>${constraints.length > 0 ? constraints.join('') : '<span style="color:var(--text-dim);">—</span>'}</div>
                            ${f.description ? `<div style="margin-top:3px;">${escapeHtml(f.description)}</div>` : ''}
                            ${f.error_hint ? `<div class="contract-error-hint"><span>Error hint:</span> ${escapeHtml(f.error_hint)}</div>` : ''}
                        </td>
                    </tr>
                `;
            }).join('')
            : '<tr><td colspan="4" style="color:var(--text-dim);">No parameters required (takes an empty object <code>{}</code>).</td></tr>';

        const failureModesHtml = failureModes.length > 0
            ? `<div class="failure-modes-list">${failureModes.map(fm => `
                <div class="failure-mode-item">
                    <div class="failure-mode-head">
                        <span class="failure-mode-code">${escapeHtml(fm.code || 'error')}</span>
                        <span class="failure-mode-cond">${escapeHtml(fm.condition || '')}</span>
                    </div>
                    ${fm.error_message ? `<div style="color:var(--text-muted);">${escapeHtml(fm.error_message)}</div>` : ''}
                    ${fm.recovery_hint ? `<div class="failure-mode-recovery">Recovery: ${escapeHtml(fm.recovery_hint)}</div>` : ''}
                </div>
            `).join('')}</div>`
            : '<div style="color:var(--text-dim);">No custom failure modes declared.</div>';

        card.innerHTML = `
            <div class="tool-contract-header">
                <div class="tool-contract-main">
                    <div class="tool-contract-title-row">
                        <span class="tool-contract-name">${escapeHtml(t.name)}</span>
                        <span class="tool-cat-badge">${escapeHtml(t.category || 'general')}</span>
                        <span class="perm-badge perm-${escapeHtml(permKey)}">${escapeHtml(formatPermissionLabel(permKey))}</span>
                        <span class="policy-badge policy-${escapeHtml(policyKey)}">${escapeHtml(formatPolicyLabel(policyKey))}</span>
                        ${t.is_custom ? '<span class="custom-tool-badge">Custom</span>' : ''}
                    </div>
                    <div class="tool-contract-desc">${escapeHtml(t.description || '')}</div>
                    ${fieldPreviewHtml}
                </div>
                <div class="tool-contract-header-right">
                    <span class="tool-timeout-chip">${escapeHtml(String(t.timeout_secs || 30))}s</span>
                    ${t.is_custom ? `<button type="button" class="btn-card-action danger btn-delete-custom-tool" data-tool-name="${escapeHtml(t.name)}">Delete</button>` : ''}
                    <button type="button" class="tool-expand-btn">${isExpanded ? 'Hide Contract ▴' : 'Contract ▾'}</button>
                </div>
            </div>
            <div class="tool-contract-body${isExpanded ? '' : ' hidden'}">
                <div>
                    <div class="contract-section-title">Parameters &amp; Validation Contract (${fields.length})</div>
                    <div class="contract-table-wrap">
                        <table class="contract-fields-table">
                            <thead>
                                <tr>
                                    <th>Field</th>
                                    <th>Type</th>
                                    <th>Rule</th>
                                    <th>Defaults, Constraints &amp; Error Hint</th>
                                </tr>
                            </thead>
                            <tbody>${fieldsRowsHtml}</tbody>
                        </table>
                    </div>
                </div>

                <details class="contract-collapsible">
                    <summary>
                        <span>Expected Output (${escapeHtml(expectedOutput.format || 'text')}) &amp; Failure Modes (${failureModes.length})</span>
                        <span>toggle details ▾</span>
                    </summary>
                    <div class="contract-collapsible-body">
                        <div class="output-contract-box">
                            <div><strong>Expected Output</strong> <span class="tool-cat-badge">${escapeHtml(expectedOutput.format || 'text')}</span></div>
                            <div style="color:var(--text-secondary);">${escapeHtml(expectedOutput.description || 'Structured or text tool output')}</div>
                            ${expectedOutput.example ? `<pre class="output-contract-example">${escapeHtml(expectedOutput.example)}</pre>` : ''}
                        </div>
                        <div>
                            <div style="margin-bottom:5px;"><strong>Failure Modes &amp; Recovery Hints</strong></div>
                            ${failureModesHtml}
                        </div>
                    </div>
                </details>

                <div class="contract-validator-box">
                    <div class="contract-section-title" style="margin-bottom:0;">Test Contract Validation</div>
                    <div class="contract-validator-row">
                        <textarea class="form-input contract-validator-input" rows="2">${escapeHtml(buildSampleArgsForContract(t))}</textarea>
                        <button type="button" class="btn-card-action primary btn-validate-contract">Validate Args</button>
                    </div>
                    <div class="contract-validator-result hidden"></div>
                </div>
            </div>
        `;

        const headerEl = card.querySelector('.tool-contract-header');
        const bodyEl = card.querySelector('.tool-contract-body');
        const expandBtn = card.querySelector('.tool-expand-btn');
        if (headerEl && bodyEl) {
            headerEl.addEventListener('click', (e) => {
                if (e.target.closest('.btn-delete-custom-tool')) return;
                const willExpand = bodyEl.classList.contains('hidden');
                bodyEl.classList.toggle('hidden', !willExpand);
                card.classList.toggle('expanded', willExpand);
                if (expandBtn) expandBtn.textContent = willExpand ? 'Hide Contract ▴' : 'Contract ▾';
                if (willExpand) expandedToolCards.add(t.name);
                else expandedToolCards.delete(t.name);
            });
        }

        const delBtn = card.querySelector('.btn-delete-custom-tool');
        if (delBtn) {
            delBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                deleteCustomToolFromUi(t.name);
            });
        }

        const valBtn = card.querySelector('.btn-validate-contract');
        const valInput = card.querySelector('.contract-validator-input');
        const valResult = card.querySelector('.contract-validator-result');
        if (valBtn && valInput && valResult) {
            valBtn.addEventListener('click', () => validateToolContractArgsFromCard(t.name, valInput, valResult));
        }

        toolsContainer.appendChild(card);
    });
}

async function loadTools() {
    initToolsModalControls();
    try {
        const res = await fetch('/api/tools');
        if (!res.ok) {
            if (toolsContainer) toolsContainer.innerHTML = `<div class="loading-state">Failed to load tools (${res.status})</div>`;
            return;
        }
        const tools = await res.json();
        cachedToolsList = Array.isArray(tools) ? tools : [];

        if (settingsToolsCountBadge) {
            settingsToolsCountBadge.textContent = `${cachedToolsList.length} tools`;
        }

        const titleEl = document.getElementById('tools-modal-title');
        if (titleEl) {
            const customCount = cachedToolsList.filter(t => t.is_custom).length;
            titleEl.textContent = customCount > 0
                ? `Tools & Contracts Registry (${cachedToolsList.length} Active · ${customCount} Custom)`
                : `Tools & Contracts Registry (${cachedToolsList.length} Active)`;
        }

        renderToolsSummaryStrip();
        renderToolsFilterPills();
        renderToolsList();
    } catch (_) {
        if (toolsContainer) toolsContainer.innerHTML = '<div class="loading-state">Failed to load plugins</div>';
    }
}

function exportSessionJson() {
    if (!currentSessionId) {
        showToast('No active session to export.', 'warning');
        return;
    }
    window.open(`/api/sessions/${encodeURIComponent(currentSessionId)}/history`, '_blank');
}

async function exportSessionMarkdown() {
    if (!currentSessionId) {
        showToast('No active session to export.', 'warning');
        return;
    }
    const title = (currentTaskBreadcrumb && currentTaskBreadcrumb.textContent.trim()) || currentSessionId;
    let events = [];
    try {
        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/history`);
        if (res.ok) events = await res.json();
    } catch (_) {}

    const lines = [`# ${title}`, ''];
    if (Array.isArray(events) && events.length > 0) {
        let assistantBuf = '';
        const flushAssistant = () => {
            if (assistantBuf.trim()) {
                lines.push('## Assistant', '', assistantBuf.trim(), '');
                assistantBuf = '';
            }
        };
        events.forEach(ev => {
            const type = ev?.type;
            const d = ev?.data || {};
            if (type === 'UserMessage') {
                flushAssistant();
                if ((d.content || '').trim()) {
                    lines.push('## User', '', d.content || '', '');
                }
            } else if (type === 'SystemCommand') {
                flushAssistant();
                const cmd = String(d.command || '').replace(/^\/+/, '');
                const args = String(d.args || '').trim();
                lines.push(`> **/${cmd}**${d.label ? ` — ${d.label}` : ''}${args ? ` \`${args}\`` : ''}`, '');
            } else if (type === 'SteeringApplied') {
                flushAssistant();
                lines.push(`> **Steer:** ${d.content || ''}`, '');
            } else if (type === 'MessageDelta') {
                assistantBuf += d.delta || '';
            } else if (type === 'IterationStep') {
                flushAssistant();
            } else if (type === 'ToolCallStarted') {
                flushAssistant();
                lines.push(`- **Tool** \`${d.tool || 'tool'}\`: \`${JSON.stringify(d.args || {})}\``);
            } else if (type === 'ToolCallFinished') {
                lines.push(`  - Result: ${d.success ? '✓ Success' : '✕ Failed'}${typeof d.duration_ms === 'number' ? ` (${d.duration_ms}ms)` : ''}`);
                lines.push('');
            }
        });
        flushAssistant();
    } else {
        streamContainer.querySelectorAll('.user-msg-bubble, .assistant-msg-bubble').forEach(b => {
            if (b.classList.contains('user-msg-bubble')) {
                lines.push('## User', '', b.dataset.rawText || b.querySelector('.user-msg-content')?.textContent || '', '');
            } else {
                const c = b.querySelector('.assistant-msg-content');
                lines.push('## Assistant', '', c ? (c.getAttribute('data-raw') || c.textContent || '') : '', '');
            }
        });
    }

    const mdContent = lines.join('\n');
    const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'conversation';
    const blob = new Blob([mdContent], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${slug}.md`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// ==========================================================================
// Token Telemetry, Analytics Popover & Context Compaction
// ==========================================================================
function formatTokenCount(num) {
    if (!num || num < 0) return '0';
    if (num >= 1000000) return `${(num / 1000000).toFixed(1)}m`;
    if (num >= 1000) return `${(num / 1000).toFixed(1)}k`;
    return num.toLocaleString();
}

function updateTokenUsageDisplay(data) {
    if (!data) return;
    const current = data.current_context_tokens || 0;
    lastCurrentContextTokens = current;
    const max = data.max_context_tokens || currentConfig?.memory?.max_context_tokens || 128000;
    const pct = max > 0 ? Math.min(100, Math.round((current / max) * 1000) / 10) : 0;
    const cost = data.estimated_cost_usd || 0.0;
    const cacheHit = data.cache_hit_rate_pct || 0.0;

    if (statCtxTokens) statCtxTokens.textContent = `${formatTokenCount(current)} / ${formatTokenCount(max)}`;
    if (statCtxPct) statCtxPct.textContent = `${pct}%`;

    if (analyticsCtxUtilText) analyticsCtxUtilText.textContent = `${current.toLocaleString()} / ${formatTokenCount(max)} (${pct}%)`;
    if (analyticsModelWindowHint) {
        analyticsModelWindowHint.textContent = `Model Window: ${formatTokenCount(max)} tokens · Automatic prompt caching active`;
    }
    if (analyticsCtxProgress) {
        analyticsCtxProgress.style.width = `${Math.min(100, pct)}%`;
        if (pct >= 85) analyticsCtxProgress.style.background = 'var(--accent-red)';
        else if (pct >= 65) analyticsCtxProgress.style.background = 'var(--accent-amber)';
        else analyticsCtxProgress.style.background = 'linear-gradient(90deg, var(--accent-blue), #60a5fa)';
    }
    if (analyticsPromptTokens) analyticsPromptTokens.textContent = (data.prompt_tokens || 0).toLocaleString();
    if (analyticsCompletionTokens) analyticsCompletionTokens.textContent = (data.completion_tokens || 0).toLocaleString();
    if (analyticsCachedTokens) analyticsCachedTokens.textContent = (data.cached_tokens || 0).toLocaleString();
    if (analyticsCacheHitRate) analyticsCacheHitRate.textContent = `${cacheHit.toFixed(1)}%`;
    if (analyticsTotalTokens) analyticsTotalTokens.textContent = (data.total_tokens || ((data.prompt_tokens || 0) + (data.completion_tokens || 0))).toLocaleString();
    if (analyticsCostUsd) analyticsCostUsd.textContent = `$${cost.toFixed(4)}`;
}

function applyMemoryCompactedStats(data) {
    if (!data || typeof data.after_tokens !== 'number') return;
    const max = currentConfig?.memory?.max_context_tokens || 128000;
    const after = data.after_tokens;
    lastCurrentContextTokens = after;
    const pct = max > 0 ? Math.min(100, Math.round((after / max) * 1000) / 10) : 0;

    if (statCtxTokens) statCtxTokens.textContent = `${formatTokenCount(after)} / ${formatTokenCount(max)}`;
    if (statCtxPct) statCtxPct.textContent = `${pct}%`;
    if (analyticsCtxUtilText) analyticsCtxUtilText.textContent = `${after.toLocaleString()} / ${formatTokenCount(max)} (${pct}%)`;
    if (analyticsModelWindowHint) {
        analyticsModelWindowHint.textContent = `Model Window: ${formatTokenCount(max)} tokens · Automatic prompt caching active`;
    }
    if (analyticsCtxProgress) {
        analyticsCtxProgress.style.width = `${Math.min(100, pct)}%`;
        if (pct >= 85) analyticsCtxProgress.style.background = 'var(--accent-red)';
        else if (pct >= 65) analyticsCtxProgress.style.background = 'var(--accent-amber)';
        else analyticsCtxProgress.style.background = 'linear-gradient(90deg, var(--accent-blue), #60a5fa)';
    }
}

function appendMemoryCompactedPill(data) {
    if (!streamContainer || !data) return;
    const before = typeof data.before_tokens === 'number' ? formatTokenCount(data.before_tokens) : '?';
    const after = typeof data.after_tokens === 'number' ? formatTokenCount(data.after_tokens) : '?';
    const folded = typeof data.messages_folded === 'number' && data.messages_folded > 0
        ? ` · ${data.messages_folded} folded`
        : '';

    const pill = document.createElement('div');
    pill.className = 'memory-compacted-pill';
    if (data.reason) pill.title = `Reason: ${data.reason}`;
    pill.innerHTML = `<span>⚡ Memory compacted: <strong>${escapeHtml(before)}</strong> → <strong>${escapeHtml(after)}</strong> tokens${escapeHtml(folded)}</span>`;
    streamContainer.appendChild(pill);
    scrollToBottom();
}

async function compactCurrentSession() {
    if (!currentSessionId) {
        if (settingsCompactStatus) settingsCompactStatus.textContent = 'No active session to compact.';
        return;
    }
    try {
        if (btnPopoverCompactCtx) btnPopoverCompactCtx.disabled = true;
        if (btnCompactActiveSession) btnCompactActiveSession.disabled = true;
        if (settingsCompactStatus) settingsCompactStatus.textContent = 'Compacting session memory...';

        const res = await fetch(`/api/sessions/${encodeURIComponent(currentSessionId)}/compact`, { method: 'POST' });

        if (btnPopoverCompactCtx) btnPopoverCompactCtx.disabled = false;
        if (btnCompactActiveSession) btnCompactActiveSession.disabled = false;

        if (res.ok) {
            const data = await res.json();
            applyMemoryCompactedStats(data);
            const beforeFmt = formatTokenCount(data.before_tokens || 0);
            const afterFmt = formatTokenCount(data.after_tokens || 0);
            const folded = data.messages_folded || 0;
            const summary = `Compacted ${beforeFmt} → ${afterFmt} tokens (${folded} folded)`;
            if (settingsCompactStatus) settingsCompactStatus.textContent = `✓ ${summary}`;

            const sseOpen = activeEventSource && activeEventSource.readyState === EventSource.OPEN;
            const changed = (data.before_tokens !== data.after_tokens) || folded > 0 || (data.events_compacted || 0) > 0;
            if (!sseOpen && changed) {
                appendMemoryCompactedPill({ ...data, reason: 'manual_compact' });
            } else if (!sseOpen && !changed) {
                appendMemoryCompactedPill({ ...data, reason: 'already_compact' });
            }
            if (tokenAnalyticsPopover) tokenAnalyticsPopover.classList.add('hidden');
        } else {
            const err = await res.json().catch(() => ({}));
            const msg = err.error || `Failed to compact context (HTTP ${res.status})`;
            if (settingsCompactStatus) settingsCompactStatus.textContent = `✕ ${msg}`;
            appendSystemWarning(`Compaction notice: ${msg}`);
        }
    } catch (e) {
        if (btnPopoverCompactCtx) btnPopoverCompactCtx.disabled = false;
        if (btnCompactActiveSession) btnCompactActiveSession.disabled = false;
        if (settingsCompactStatus) settingsCompactStatus.textContent = `✕ ${e.message}`;
        appendSystemWarning(`Compaction error: ${e.message}`);
    }
}

function appendSystemNotice(msg) {
    const el = document.createElement('div');
    el.className = 'code-block-card';
    el.style.borderColor = 'var(--accent-blue)';
    el.innerHTML = `
        <div class="code-block-header" style="color: var(--accent-blue);"><span>Context &amp; Telemetry Notice</span></div>
        <div class="code-block-body" style="color: #93c5fd;">${escapeHtml(msg)}</div>
    `;
    streamContainer.appendChild(el);
    scrollToBottom();
}

// ==========================================================================
// Zero-Dependency Markdown Renderer (Tables, Code Blocks, Highlights, Paths)
// ==========================================================================
function renderMarkdown(md) {
    if (!md) return '';
    const lines = md.split('\n');
    let html = '';
    let inTable = false;
    let tableHeaders = [];
    let tableAlignments = [];
    let inList = false;
    let listType = 'ul';
    let inCodeBlock = false;
    let codeBlockLang = '';
    let codeBlockContent = [];

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trim();

        if (trimmed.startsWith('```')) {
            if (!inCodeBlock) {
                closeList();
                if (inTable) {
                    inTable = false;
                    html += '</tbody></table></div>';
                }
                inCodeBlock = true;
                codeBlockLang = trimmed.substring(3).trim();
                codeBlockContent = [];
                continue;
            } else {
                inCodeBlock = false;
                const langLabel = codeBlockLang ? escapeHtml(codeBlockLang) : 'code';
                const codeEscaped = escapeHtml(codeBlockContent.join('\n'));
                html += `
                    <div class="code-block-card">
                        <div class="code-block-header">
                            <span class="code-lang">${langLabel}</span>
                            <button type="button" class="code-btn-action" data-copy-code="1">Copy</button>
                        </div>
                        <pre class="code-block-body" style="margin: 0;"><code class="code-inner">${codeEscaped}</code></pre>
                    </div>
                `;
                continue;
            }
        }

        if (inCodeBlock) {
            codeBlockContent.push(line);
            continue;
        }

        if (trimmed.startsWith('|') && trimmed.endsWith('|')) {
            closeList();
            const cells = trimmed.split('|').slice(1, -1).map(c => c.trim());
            const isAlignRow = cells.every(c => /^:?-+:?$/.test(c));

            if (isAlignRow) {
                tableAlignments = cells.map(c => {
                    const left = c.startsWith(':');
                    const right = c.endsWith(':');
                    if (left && right) return 'center';
                    if (right) return 'right';
                    return 'left';
                });
                continue;
            }

            if (!inTable) {
                inTable = true;
                tableHeaders = cells;
                tableAlignments = [];
                html += '<div class="table-responsive"><table class="md-table"><thead><tr>';
                tableHeaders.forEach(h => { html += `<th>${formatInline(h)}</th>`; });
                html += '</tr></thead><tbody>';
            } else {
                html += '<tr>';
                cells.forEach((c, idx) => {
                    const align = (tableAlignments && tableAlignments[idx]) || 'left';
                    html += `<td style="text-align: ${align}">${formatInline(c)}</td>`;
                });
                html += '</tr>';
            }
            continue;
        } else if (inTable) {
            inTable = false;
            html += '</tbody></table></div>';
        }

        if (trimmed.startsWith('### ')) {
            closeList();
            html += `<h3>${formatInline(trimmed.substring(4))}</h3>`;
            continue;
        }
        if (trimmed.startsWith('## ')) {
            closeList();
            html += `<h2>${formatInline(trimmed.substring(3))}</h2>`;
            continue;
        }
        if (trimmed.startsWith('# ')) {
            closeList();
            html += `<h1>${formatInline(trimmed.substring(2))}</h1>`;
            continue;
        }

        if (trimmed === '---' || trimmed === '***' || trimmed === '___') {
            closeList();
            html += '<hr style="border: 0; border-top: 1px solid var(--border-subtle); margin: 12px 0;">';
            continue;
        }

        if (trimmed.startsWith('- ') || trimmed.startsWith('* ') || trimmed.startsWith('• ')) {
            if (!inList) {
                inList = true;
                listType = 'ul';
                html += '<ul>';
            }
            html += `<li>${formatInline(trimmed.substring(2))}</li>`;
            continue;
        }

        const olMatch = trimmed.match(/^(\d+)\.\s+(.*)/);
        if (olMatch) {
            if (!inList) {
                inList = true;
                listType = 'ol';
                html += '<ol>';
            }
            html += `<li>${formatInline(olMatch[2])}</li>`;
            continue;
        }

        closeList();

        if (trimmed.startsWith('> ')) {
            html += `<blockquote style="border-left: 3px solid var(--accent-blue); padding-left: 10px; color: var(--text-secondary); margin: 8px 0;">${formatInline(trimmed.substring(2))}</blockquote>`;
            continue;
        }

        if (trimmed === '') continue;

        html += `<p>${formatInline(line)}</p>`;
    }

    if (inCodeBlock) {
        const langLabel = codeBlockLang ? escapeHtml(codeBlockLang) : 'code';
        const codeEscaped = escapeHtml(codeBlockContent.join('\n'));
        html += `
            <div class="code-block-card">
                <div class="code-block-header">
                    <span class="code-lang">${langLabel}</span>
                    <button type="button" class="code-btn-action" data-copy-code="1">Copy</button>
                </div>
                <pre class="code-block-body" style="margin: 0;"><code class="code-inner">${codeEscaped}</code></pre>
            </div>
        `;
    }

    if (inTable) html += '</tbody></table></div>';
    closeList();

    function closeList() {
        if (inList) {
            html += listType === 'ul' ? '</ul>' : '</ol>';
            inList = false;
        }
    }
    return html;
}

function formatInline(text) {
    if (!text) return '';
    let res = escapeHtml(text);
    const placeholders = [];
    const hold = (html) => {
        const key = `\x00H${placeholders.length}\x00`;
        placeholders.push(html);
        return key;
    };
    const fileIconSvg = '<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>';

    res = res.replace(/`([^`]+)`/g, (match, p1) => {
        const clean = p1.trim();
        if (/^(?:[\w.-]+[\\/])+[\w.-]+\.\w+$|^[\w-]+\.(?:rs|toml|lock|json|md|html|css|js|ts|py|sh|yaml|yml)$/.test(clean)) {
            return hold(`<span class="md-path-link" title="Click to copy path" data-copy-path="${clean}">${fileIconSvg}<span>${clean}</span></span>`);
        }
        return hold(`<code>${p1}</code>`);
    });

    res = res.replace(/\[(.*?)\]\((.*?)\)/g, (match, label, url) => {
        const normalizedUrl = url.trim().replace(/&amp;/g, '&');
        if (/^(?:https?:\/\/|\/|#)/i.test(normalizedUrl)) {
            return hold(`<a href="${url.trim()}" target="_blank" rel="noopener">${label}</a>`);
        }
        return hold(label);
    });

    res = res.replace(/(^|[\s(])((?:[a-zA-Z0-9_\-.]+[\\/])+[a-zA-Z0-9_\-.]+\.[a-zA-Z0-9_]+|[a-zA-Z0-9_\-.]+\.(?:rs|toml|lock|json|md|html|css|js|ts|py|sh|yaml|yml))(?=[.,;:)]?(?:\s|$))/g,
        (match, pre, pathStr) => {
            return `${pre}${hold(`<span class="md-path-link" title="Click to copy path" data-copy-path="${pathStr}">${fileIconSvg}<span>${pathStr}</span></span>`)}`;
        });

    res = res.replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>');
    res = res.replace(/==([^=]+)==/g, '<mark class="md-highlight">$1</mark>');
    res = res.replace(/~~([^~]+)~~/g, '<del>$1</del>');
    res = res.replace(/\x00H(\d+)\x00/g, (_, idx) => placeholders[Number(idx)]);
    return res;
}

window.copyPath = function(el, pathArg) {
    const path = pathArg || el?.getAttribute?.('data-copy-path') || '';
    if (!path || !el || el.dataset.copying === '1') return;
    navigator.clipboard.writeText(path).then(() => {
        el.dataset.copying = '1';
        const orig = el.innerHTML;
        el.textContent = '✓ Copied';
        setTimeout(() => {
            el.innerHTML = orig;
            delete el.dataset.copying;
        }, 1400);
    }).catch(() => {});
};

window.copyCodeBlock = function(btn) {
    const card = btn.closest('.code-block-card');
    if (!card) return;
    const code = card.querySelector('code');
    if (!code) return;
    navigator.clipboard.writeText(code.innerText || code.textContent).then(() => {
        const orig = btn.textContent;
        btn.textContent = '✓ Copied';
        setTimeout(() => { btn.textContent = orig; }, 1500);
    }).catch(() => {});
};

window.copyCode = function(btn) {
    const card = btn.closest('.code-block-card, .tool-card');
    if (!card) return;
    const code = card.querySelector('code');
    if (!code) return;
    navigator.clipboard.writeText(code.textContent).then(() => {
        const orig = btn.textContent;
        btn.textContent = '✓';
        setTimeout(() => { btn.textContent = orig; }, 1500);
    }).catch(() => {});
};

function escapeHtml(str) {
    if (str === null || str === undefined) return '';
    return String(str)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#039;');
}

// ==========================================================================
// Automations (first-class scheduled agent runs)
// ==========================================================================
let allAutomations = [];

async function fetchAutomations() {
    try {
        const res = await fetch('/api/automations');
        if (!res.ok) return;
        const data = await res.json();
        allAutomations = Array.isArray(data.automations) ? data.automations : [];
        renderAutomations();
    } catch (_) { /* offline: keep previous list */ }
}

// Which automation the editor is editing (null = creating a new one).
let editingAutomationId = null;
let automationLogs = [];
let automationLogFilterId = null;

function automationRowStatus(a) {
    if (a.is_running) return { text: 'Running', cls: 'badge-running' };
    if (!a.enabled) return { text: 'Paused', cls: 'badge-idle' };
    if (a.last_status === 'failed') return { text: 'Failed', cls: 'badge-danger' };
    if (a.last_status === 'stopped') return { text: 'Stopped', cls: 'badge-idle' };
    return { text: 'Idle', cls: 'badge-active' };
}

async function openAutomationLogs(automationId) {
    automationLogFilterId = automationId;
    const modal = document.getElementById('automation-logs-modal');
    if (!modal) return;
    const title = document.getElementById('automation-logs-title');
    const nameOf = (allAutomations || []).find(a => a.id === automationId);
    if (title) {
        title.textContent = nameOf ? `${nameOf.name} - Run Log` : 'Automation Run Log';
    }
    modal.classList.remove('hidden');
    await refreshAutomationLogs();
}

function closeAutomationLogs() {
    document.getElementById('automation-logs-modal')?.classList.add('hidden');
    automationLogFilterId = null;
}

async function refreshAutomationLogs() {
    const body = document.getElementById('automation-logs-body');
    const summary = document.getElementById('automation-logs-summary');
    if (!body) return;
    body.innerHTML = '<div class="automation-log-empty">Loading run log...</div>';
    try {
        const q = automationLogFilterId ? `?automation_id=${encodeURIComponent(automationLogFilterId)}` : '';
        const res = await fetch(`/api/automation/logs${q}`);
        if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
        const data = await res.json();
        automationLogs = data.logs || [];
        const one = automationLogFilterId
            ? await (await fetch(`/api/automations/${encodeURIComponent(automationLogFilterId)}/logs`)).json().catch(() => null)
            : null;
        renderAutomationLogs(automationLogs, one);
    } catch (e) {
        body.innerHTML = `<div class="automation-log-empty" style="color:var(--accent-red)">Failed to load run log: ${escapeHtml(e.message)}</div>`;
    }
}

function renderAutomationLogs(logs, detail) {
    const body = document.getElementById('automation-logs-body');
    const summary = document.getElementById('automation-logs-summary');
    if (!body) return;
    body.innerHTML = '';
    if (summary) {
        if (!logs.length) {
            summary.textContent = 'No runs yet.';
        } else {
            const ok = logs.filter(l => l.status === 'success').length;
            const bad = logs.filter(l => l.status === 'failed').length;
            const stopped = logs.filter(l => l.status === 'stopped').length;
            const avg = detail && detail.summary
                ? `${Math.round(detail.summary.avg_duration_ms / 1000)}s avg`
                : null;
            const parts = [`${logs.length} run${logs.length === 1 ? '' : 's'} loaded`, `${ok} ok`];
            if (bad) parts.push(`${bad} failed`);
            if (stopped) parts.push(`${stopped} stopped`);
            if (avg) parts.push(avg);
            summary.textContent = parts.join(' - ');
        }
    }
    if (!logs.length) {
        const empty = document.createElement('div');
        empty.className = 'automation-log-empty';
        empty.textContent = 'This automation has not run yet. Run it once and the output will appear here - never in your conversations.';
        body.appendChild(empty);
        return;
    }
    logs.forEach(log => {
        const row = document.createElement('div');
        row.className = `automation-log-row status-${log.status}`;
        const head = document.createElement('div');
        head.className = 'automation-log-head';
        const status = document.createElement('span');
        status.className = `automation-log-status ${log.status}`;
        status.textContent = log.status;
        const title = document.createElement('strong');
        title.textContent = log.automation_name || log.automation_id;
        const when = document.createElement('span');
        when.className = 'automation-log-time';
        const started = new Date(log.started_at);
        when.textContent = started.toLocaleString() + (log.duration_ms != null ? ` - ${(log.duration_ms / 1000).toFixed(1)}s` : '');
        head.append(status, title, when);

        const meta = document.createElement('div');
        meta.className = 'automation-log-meta';
        const bits = [];
        if (log.trigger) bits.push(`trigger: ${log.trigger}`);
        if (log.model) bits.push(`model: ${log.model}`);
        if (log.tool_calls) bits.push(`${log.tool_calls} tool calls`);
        if (log.tokens) bits.push(`${log.tokens} tokens`);
        meta.textContent = bits.join(' - ');

        const out = document.createElement('pre');
        out.className = 'automation-log-output';
        out.textContent = log.error ? `Error: ${log.error}` : (log.output || '(no output)');
        row.append(head, meta, out);
        body.appendChild(row);
    });
}

function renderAutomations() {
    const list = document.getElementById('automations-list');
    if (!list) return;
    list.innerHTML = '';

    if (allAutomations.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'automation-item';
        empty.textContent = 'No automations yet. Create one to run the agent on a schedule.';
        empty.style.cssText = 'opacity:.7;font-size:0.78rem';
        list.appendChild(empty);
        return;
    }

    allAutomations.forEach(a => {
        const item = document.createElement('div');
        item.className = 'automation-item';

        const meta = document.createElement('div');
        meta.className = 'automation-meta';
        const name = document.createElement('strong');
        name.textContent = a.name;
        const sub = document.createElement('span');
        const parts = [a.trigger_label];
        if (a.project_id) parts.push(`project: ${a.project_id}`);
        if (a.run_count) parts.push(`${a.run_count} run(s)`);
        if (a.last_status && a.last_status !== 'idle') parts.push(a.last_status);
        if (a.is_running) parts.push('in progress');
        sub.textContent = parts.join(' · ');
        meta.append(name, sub);

        const actions = document.createElement('div');
        actions.style.cssText = 'display:flex;gap:6px;align-items:center';

        const badge = document.createElement('span');
        const status = automationRowStatus(a);
        badge.className = status.cls;
        badge.textContent = status.text;

        const runBtn = document.createElement('button');
        runBtn.type = 'button';
        runBtn.className = 'btn-card-action';
        runBtn.textContent = 'Run';
        runBtn.onclick = async () => {
            runBtn.disabled = true;
            runBtn.textContent = 'Starting...';
            try {
                const res = await fetch(`/api/automations/${encodeURIComponent(a.id)}/run`, { method: 'POST' });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                const data = await res.json().catch(() => ({}));
                showToast(`Started '${a.name}' - output will appear in its run log.`, 'success', 6000);
                fetchAutomations();
                // Open the run log so the user can watch it, like a job view.
                if (data.run_id) {
                    automationLogFilterId = a.id;
                }
            } catch (e) {
                showToast('Failed to run automation: ' + e.message, 'error', 7000);
            } finally {
                runBtn.disabled = false;
                runBtn.textContent = 'Run';
            }
        };

        // Stop only appears while the automation is actually running.
        const stopBtn = document.createElement('button');
        stopBtn.type = 'button';
        stopBtn.className = 'btn-card-action danger';
        stopBtn.textContent = 'Stop';
        stopBtn.style.display = a.is_running ? '' : 'none';
        stopBtn.onclick = async () => {
            stopBtn.disabled = true;
            try {
                const res = await fetch(`/api/automations/${encodeURIComponent(a.id)}/stop`, { method: 'POST' });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                showToast(`Stopping '${a.name}'...`, 'info');
                fetchAutomations();
            } catch (e) {
                showToast('Failed to stop automation: ' + e.message, 'error', 7000);
                stopBtn.disabled = false;
            }
        };

        const toggleBtn = document.createElement('button');
        toggleBtn.type = 'button';
        toggleBtn.className = 'btn-card-action';
        toggleBtn.textContent = a.enabled ? 'Pause' : 'Enable';
        toggleBtn.onclick = async () => {
            try {
                const res = await fetch('/api/automations', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ ...a, enabled: !a.enabled })
                });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                fetchAutomations();
            } catch (e) {
                showToast('Failed to update automation: ' + e.message, 'error', 7000);
            }
        };

        const delBtn = document.createElement('button');
        delBtn.type = 'button';
        delBtn.className = 'btn-card-action danger';
        delBtn.textContent = 'Delete';
        delBtn.onclick = async () => {
            const ok = await uiConfirm(`Delete automation '${a.name}'?`, {
                title: 'Delete Automation', confirmText: 'Delete', danger: true
            });
            if (!ok) return;
            try {
                const res = await fetch(`/api/automations/${encodeURIComponent(a.id)}`, { method: 'DELETE' });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                showToast(`Deleted '${a.name}'.`, 'info');
                fetchAutomations();
            } catch (e) {
                showToast('Failed to delete automation: ' + e.message, 'error', 7000);
            }
        };

        const logsBtn = document.createElement('button');
        logsBtn.type = 'button';
        logsBtn.className = 'btn-card-action';
        logsBtn.textContent = 'Logs';
        logsBtn.title = 'Run output for this automation (not conversations)';
        logsBtn.onclick = () => openAutomationLogs(a.id);

        const editBtn = document.createElement('button');
        editBtn.type = 'button';
        editBtn.className = 'btn-card-action';
        editBtn.textContent = 'Edit';
        editBtn.onclick = () => showAutomationEditor(a.id);

        actions.append(badge, runBtn, stopBtn, logsBtn, editBtn, toggleBtn, delBtn);
        item.append(meta, actions);
        list.appendChild(item);
    });
}

function showAutomationEditor(automationId = null) {
    const editor = document.getElementById('automation-editor');
    if (!editor) return;
    editor.classList.remove('hidden');
    // Remember what we are editing: null = create a new automation.
    editingAutomationId = automationId;
    const existing = automationId ? (allAutomations || []).find(a => a.id === automationId) : null;
    const saveBtn = document.getElementById('btn-save-automation');
    if (saveBtn) saveBtn.textContent = existing ? 'Save Changes' : 'Save Automation';

    const projectSel = document.getElementById('automation-project');
    if (projectSel) {
        projectSel.innerHTML = '<option value="">Standalone (no project)</option>';
        (allProjects || []).forEach(p => {
            const opt = document.createElement('option');
            opt.value = p.id;
            opt.textContent = p.name;
            projectSel.appendChild(opt);
        });
        projectSel.value = existing && existing.project_id ? existing.project_id : '';
    }

    // Prefill the form so any automation can be edited after it was made.
    if (document.getElementById('automation-name')) document.getElementById('automation-name').value = existing ? existing.name : '';
    if (document.getElementById('automation-description')) document.getElementById('automation-description').value = existing ? (existing.description || '') : '';
    if (document.getElementById('automation-prompt')) document.getElementById('automation-prompt').value = existing ? existing.prompt : '';
    const triggerSel = document.getElementById('automation-trigger');
    const everyEl = document.getElementById('automation-every');
    const cronEl = document.getElementById('automation-cron');
    let kind = 'manual';
    let every = '';
    let cron = '';
    if (existing && existing.trigger) {
        kind = existing.trigger.kind || 'manual';
        if (kind === 'interval') every = `${existing.trigger.every_secs || 1800}s`;
        if (kind === 'cron') cron = existing.trigger.expr || '';
    }
    if (triggerSel) triggerSel.value = kind;
    if (everyEl) everyEl.value = every;
    if (cronEl) cronEl.value = cron;
    document.getElementById('automation-name')?.focus();
}

function hideAutomationEditor() {
    const editor = document.getElementById('automation-editor');
    if (editor) editor.classList.add('hidden');
    editingAutomationId = null;
    const saveBtn = document.getElementById('btn-save-automation');
    if (saveBtn) saveBtn.textContent = 'Save Automation';
}

async function saveAutomationFromForm() {
    const name = document.getElementById('automation-name').value.trim();
    const description = document.getElementById('automation-description').value.trim();
    const prompt = document.getElementById('automation-prompt').value.trim();
    const triggerKind = document.getElementById('automation-trigger').value;
    const every = document.getElementById('automation-every').value.trim();
    const cron = document.getElementById('automation-cron').value.trim();
    const projectId = document.getElementById('automation-project').value;

    if (!name || !prompt) {
        showToast('Automation name and prompt are required.', 'warning');
        return;
    }

    let trigger = { kind: 'manual' };
    if (triggerKind === 'interval') {
        const secs = parseEveryToSecs(every) || 1800;
        trigger = { kind: 'interval', every_secs: secs };
    } else if (triggerKind === 'cron') {
        trigger = { kind: 'cron', expr: cron || '*/10 * * * *' };
    } else if (triggerKind === 'always_on') {
        trigger = { kind: 'always_on' };
    }

    const payload = {
        name,
        description,
        prompt,
        trigger,
        enabled: true,
        project_id: projectId || null
    };

    try {
        let res;
        if (editingAutomationId) {
            // PATCH only the edited fields; the server preserves the rest
            // (including run history).
            res = await fetch(`/api/automations/${encodeURIComponent(editingAutomationId)}`, {
                method: 'PATCH',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload)
            });
        } else {
            res = await fetch('/api/automations', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload)
            });
        }
        if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
        showToast(editingAutomationId ? `Updated automation '${name}'.` : `Saved automation '${name}'.`, 'success');
        hideAutomationEditor();
        fetchAutomations();
    } catch (e) {
        showToast('Failed to save automation: ' + e.message, 'error', 7000);
    }
}

function parseEveryToSecs(raw) {
    if (!raw) return null;
    const m = /^(\d+)\s*([smhd])$/i.exec(raw.trim());
    if (!m) return null;
    const n = parseInt(m[1], 10);
    const mult = { s: 1, m: 60, h: 3600, d: 86400 }[m[2].toLowerCase()] || 60;
    return n * mult;
}

document.addEventListener('DOMContentLoaded', () => {
    const newBtn = document.getElementById('btn-new-automation');
    if (newBtn) newBtn.addEventListener('click', () => showAutomationEditor(null));
    const closeLogsBtn = document.getElementById('btn-close-automation-logs');
    if (closeLogsBtn) closeLogsBtn.addEventListener('click', closeAutomationLogs);
    const doneLogsBtn = document.getElementById('btn-done-automation-logs');
    if (doneLogsBtn) doneLogsBtn.addEventListener('click', closeAutomationLogs);
    const refreshLogsBtn = document.getElementById('btn-refresh-automation-logs');
    if (refreshLogsBtn) refreshLogsBtn.addEventListener('click', refreshAutomationLogs);
    // Clicking the backdrop of the run-log modal dismisses it.
    const logsModal = document.getElementById('automation-logs-modal');
    if (logsModal) {
        logsModal.addEventListener('click', (e) => {
            if (e.target === logsModal) closeAutomationLogs();
        });
    }
    const cancelBtn = document.getElementById('btn-cancel-automation');
    if (cancelBtn) cancelBtn.addEventListener('click', hideAutomationEditor);
    const saveBtn = document.getElementById('btn-save-automation');
    if (saveBtn) saveBtn.addEventListener('click', saveAutomationFromForm);
    fetchAutomations();
});
// Sidebar automations category
function renderSidebarAutomations() {
    const list = document.getElementById('sidebar-automations-list');
    if (!list) return;
    list.innerHTML = '';
    if (!allAutomations || allAutomations.length === 0) {
        const empty = document.createElement('div');
        empty.style.cssText = 'padding:6px 10px;font-size:0.72rem;color:var(--text-dim);';
        empty.textContent = 'No automations yet';
        list.appendChild(empty);
        return;
    }
    allAutomations.forEach(a => {
        // A row per automation: name + live status, opening its run log.
        const row = document.createElement('button');
        row.type = 'button';
        row.className = 'nav-item-link';
        row.style.cssText = 'display:flex;align-items:center;gap:8px;width:100%;text-align:left;';
        const running = !!a.is_running;
        const dot = document.createElement('span');
        dot.textContent = running ? '\u25D0' : (a.enabled ? '\u25CF' : '\u25CB');
        dot.style.cssText = 'font-size:0.7rem;color:' + (running ? '#38bdf8' : (a.enabled ? '#3fb950' : '#8b949e'));
        const label = document.createElement('span');
        label.textContent = a.name;
        label.style.cssText = 'flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
        const when = document.createElement('span');
        when.textContent = running ? 'running' : (a.trigger_label || '');
        when.style.cssText = 'font-size:0.68rem;color:' + (running ? '#38bdf8' : 'var(--text-dim)');
        row.append(dot, label, when);
        const status = automationRowStatus(a);
        row.title = a.name + ' - ' + (a.trigger_label || '') + (a.project_id ? ' (project: ' + a.project_id + ')' : '') + '\nStatus: ' + status.text;
        // Clicking opens the run log: automation output never becomes a conversation.
        row.onclick = () => openAutomationLogs(a.id);
        list.appendChild(row);
    });
}

// Keep the sidebar list in sync whenever automations are (re)fetched.
const __origRenderAutomations = renderAutomations;
renderAutomations = function () {
    __origRenderAutomations();
    renderSidebarAutomations();
};

document.addEventListener('DOMContentLoaded', () => {
    const btn = document.getElementById('btn-sidebar-new-automation');
    if (btn) {
        btn.addEventListener('click', () => {
            // The settings hub owns the automation editor, so open it on the
            // Skills & Automations page before revealing the form.
            if (typeof openSettingsModal === 'function') openSettingsModal('skills');
            if (typeof showAutomationEditor === 'function') showAutomationEditor();
        });
    }
});
// ==========================================================================
// First-run onboarding
// ==========================================================================
let onboardingState = { step: 1, total: 3, info: null };

const ONBOARDING_PRESETS = {
    openai:     { base: 'https://api.openai.com/v1',        model: 'gpt-4o-mini',  keyLabel: 'sk-...' },
    deepseek:   { base: 'https://api.deepseek.com/v1',      model: 'deepseek-chat', keyLabel: 'sk-...' },
    openrouter: { base: 'https://openrouter.ai/api/v1',     model: 'openai/gpt-4o-mini', keyLabel: 'sk-or-...' },
    groq:       { base: 'https://api.groq.com/openai/v1',   model: 'llama-3.3-70b-versatile', keyLabel: 'gsk_...' },
    nvidia:     { base: 'https://integrate.api.nvidia.com/v1', model: 'nvidia/llama-3.1-nemotron-70b-instruct', keyLabel: 'nvapi-...' },
    ollama:     { base: 'http://localhost:11434/v1',        model: 'llama3.3',     keyLabel: 'not required' },
    vllm:       { base: 'http://localhost:8000/v1',         model: 'default',      keyLabel: 'not required' }
};

async function checkOnboarding() {
    try {
        const res = await fetch('/api/onboarding');
        if (!res.ok) return;
        const info = await res.json();
        onboardingState.info = info;
        if (!info.onboarded) {
            openOnboarding();
        }
    } catch (_) { /* offline: skip */ }
}

function openOnboarding() {
    const modal = document.getElementById('onboarding-modal');
    if (!modal) return;
    onboardingState.step = 1;
    const pathEl = document.getElementById('onboarding-project-path');
    if (pathEl && onboardingState.info) pathEl.value = onboardingState.info.project_path || '';
    renderOnboardingStep();
    modal.classList.remove('hidden');
    // Escape dismisses the wizard (it is completable later from Settings).
    modal.dataset.dismissable = '1';
    // The wizard is the first thing a new user sees: it must take focus.
    requestAnimationFrame(() => focusOnboardingFirstControl());
}

/// Focus the first enabled control of the current step so keyboard users do not
/// have to hunt for it (the wizard used to open with focus on nothing).
function focusOnboardingFirstControl() {
    const modal = document.getElementById('onboarding-modal');
    if (!modal || modal.classList.contains('hidden')) return;
    const step = document.getElementById(`onboarding-step-${onboardingState.step}`);
    const host = step || modal;
    const target =
        host.querySelector('input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), button:not([disabled])') ||
        modal.querySelector('button:not([disabled])');
    if (target && typeof target.focus === 'function') target.focus();
}

function renderOnboardingStep() {
    const s = onboardingState.step;
    for (let i = 1; i <= onboardingState.total; i++) {
        const el = document.getElementById(`onboarding-step-${i}`);
        if (el) el.classList.toggle('hidden', i !== s);
    }
    // Keep focus useful as the steps advance.
    requestAnimationFrame(() => focusOnboardingFirstControl());
    const titles = ['Welcome to Harness', 'Connect a Model Provider', 'Your Workspace'];
    const titleEl = document.getElementById('onboarding-title');
    if (titleEl) titleEl.textContent = titles[s - 1] || 'Welcome to Harness';
    const iconEl = document.getElementById('onboarding-step-icon');
    if (iconEl) iconEl.textContent = String(s);
    const progEl = document.getElementById('onboarding-progress');
    if (progEl) progEl.textContent = `Step ${s} of ${onboardingState.total}`;
    const backEl = document.getElementById('btn-onboarding-back');
    if (backEl) backEl.disabled = s === 1;
    const nextEl = document.getElementById('btn-onboarding-next');
    if (nextEl) nextEl.textContent = s === onboardingState.total ? 'Finish' : 'Continue';
}

function onboardingPresetChanged() {
    const sel = document.getElementById('onboarding-preset');
    if (!sel) return;
    const preset = ONBOARDING_PRESETS[sel.value];
    if (!preset) return;
    const baseEl = document.getElementById('onboarding-api-base');
    const modelEl = document.getElementById('onboarding-model');
    const keyEl = document.getElementById('onboarding-api-key');
    if (baseEl) baseEl.value = preset.base;
    if (modelEl && !modelEl.value) modelEl.value = preset.model;
    if (keyEl) keyEl.placeholder = preset.keyLabel;
}

async function onboardingNext() {
    if (onboardingState.step === 2) {
        // Save the provider if one was entered.
        const base = (document.getElementById('onboarding-api-base')?.value || '').trim();
        const key = (document.getElementById('onboarding-api-key')?.value || '').trim();
        const model = (document.getElementById('onboarding-model')?.value || '').trim();
        const status = document.getElementById('onboarding-provider-status');
        if (base) {
            // A remote provider with no key cannot work, and a wrong key only
            // fails on the user's first prompt. Verify it before the wizard
            // stores it, so a typo is fixed here rather than post-onboarding.
            const isLocal = base.includes('localhost') || base.includes('127.0.0.1');
            if (key && !isLocal) {
                const works = await onboardingTestKey();
                if (!works) return; // stay on this step
            }
            const id = (document.getElementById('onboarding-preset')?.value || 'custom') + '-' + Date.now().toString(36);
            if (!currentConfig) { currentConfig = await (await fetch('/api/config')).json(); }
            if (!currentConfig.custom_providers) currentConfig.custom_providers = [];
            currentConfig.custom_providers.push({
                id, name: (document.getElementById('onboarding-preset')?.value || 'Custom'),
                api_base: base, api_key: key, default_model: model || 'default',
                is_active: true, enabled: true
            });
            currentConfig.provider = id;
            currentConfig.api_base = base;
            currentConfig.api_key = key;
            if (model) currentConfig.model = model;
            try {
                const result = await saveCurrentConfig();
                if (result.ok) {
                    if (status) { status.textContent = 'Provider saved.'; status.style.color = '#3fb950'; }
                }
            } catch (e) {
                if (status) { status.textContent = 'Could not save provider: ' + e.message; status.style.color = '#f85149'; }
                return; // stay on this step so the user can fix it
            }
        }
    }

    if (onboardingState.step === 3) {
        // Persist the project path if it changed.
        const pathEl = document.getElementById('onboarding-project-path');
        const extraEl = document.getElementById('onboarding-extra-paths');
        const newPath = (pathEl?.value || '').trim();
        const extras = (extraEl?.value || '').split('\n').map(s => s.trim()).filter(Boolean);
        if (currentConfig && currentConfig.projects && currentConfig.projects.length > 0) {
            const active = currentConfig.projects.find(p => p.id === currentConfig.active_project_id) || currentConfig.projects[0];
            if (active && (newPath || extras.length)) {
                try {
                    const body = { defaults: active.defaults || {} };
                    if (newPath) body.path = newPath;
                    if (extras.length) body.extra_paths = extras;
                    await fetch('/api/projects/' + encodeURIComponent(active.id) + '/defaults', {
                        method: 'PATCH',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify(body)
                    });
                } catch (_) { /* non-fatal */ }
            }
        }
        await finishOnboarding();
        return;
    }

    onboardingState.step = Math.min(onboardingState.total, onboardingState.step + 1);
    renderOnboardingStep();
}

function onboardingBack() {
    onboardingState.step = Math.max(1, onboardingState.step - 1);
    renderOnboardingStep();
}

/// "Test key": actually call the provider before the wizard is allowed to finish.
///
/// The wizard used to accept any typed key, so a typo (or an expired key) only
/// surfaced as a failed first message with no explanation. This reuses the same
/// `/api/models/remote` probe the Settings provider card uses.
async function onboardingTestKey() {
    const base = (document.getElementById('onboarding-api-base')?.value || '').trim();
    const key = (document.getElementById('onboarding-api-key')?.value || '').trim();
    const model = (document.getElementById('onboarding-model')?.value || '').trim();
    const status = document.getElementById('onboarding-test-status');
    const btn = document.getElementById('btn-onboarding-test-key');

    if (!base) {
        if (status) { status.textContent = 'Enter an API Base URL first.'; status.style.color = '#f85149'; }
        return;
    }
    const originalLabel = btn ? btn.textContent : '';
    if (btn) { btn.disabled = true; btn.textContent = 'Testing...'; }
    if (status) { status.textContent = `Connecting to ${base}/models ...`; status.style.color = 'var(--text-dim)'; }

    try {
        const res = await fetch('/api/models/remote', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ api_base: base, api_key: key })
        });
        if (!res.ok) {
            const detail = await res.text().catch(() => '');
            throw new Error(detail || `Status ${res.status}`);
        }
        const models = await res.json();
        const count = Array.isArray(models) ? models.length : 0;
        if (status) {
            status.textContent = `Key works - ${count} model${count === 1 ? '' : 's'} available.`;
            status.style.color = '#3fb950';
        }
        if (btn) btn.textContent = 'Key works';
        // Adopt the first available model when the user left the field empty,
        // exactly like the Settings card does.
        if (count > 0 && !model) {
            const modelEl = document.getElementById('onboarding-model');
            if (modelEl) modelEl.value = models[0].id;
        }
        return true;
    } catch (e) {
        if (status) {
            status.textContent = `Key test failed: ${e.message}`;
            status.style.color = '#f85149';
        }
        if (btn) btn.textContent = 'Test failed';
        return false;
    } finally {
        if (btn) { btn.disabled = false; btn.textContent = btn.textContent || originalLabel; }
    }
}

async function finishOnboarding(skipped = false) {
    try {
        await fetch('/api/onboarding/complete', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ skipped })
        });
    } catch (_) { /* best effort */ }
    const modal = document.getElementById('onboarding-modal');
    if (modal) modal.classList.add('hidden');
    // Refresh everything the wizard may have changed.
    try {
        currentConfig = await (await fetch('/api/config')).json();
        if (typeof renderConfiguredProviders === 'function') renderConfiguredProviders();
        if (typeof fetchProjects === 'function') await fetchProjects();
        if (typeof fetchRemoteModels === 'function') fetchRemoteModels();
        if (typeof updateModelPillDisplay === 'function') updateModelPillDisplay();
    } catch (_) { /* ignore */ }
    showToast(skipped ? 'Onboarding skipped — add a provider in Settings when ready.' : 'You are all set.', 'success', 6000);
}

document.addEventListener('DOMContentLoaded', () => {
    const nextBtn = document.getElementById('btn-onboarding-next');
    if (nextBtn) nextBtn.addEventListener('click', onboardingNext);
    const backBtn = document.getElementById('btn-onboarding-back');
    if (backBtn) backBtn.addEventListener('click', onboardingBack);
    const skipBtn = document.getElementById('btn-onboarding-skip');
    if (skipBtn) skipBtn.addEventListener('click', () => finishOnboarding(true));
    const presetSel = document.getElementById('onboarding-preset');
    if (presetSel) presetSel.addEventListener('change', onboardingPresetChanged);
    const testBtn = document.getElementById('btn-onboarding-test-key');
    if (testBtn) testBtn.addEventListener('click', onboardingTestKey);
    const modal = document.getElementById('onboarding-modal');
    if (modal) {
        // Escape dismisses the wizard (it can be completed later from Settings).
        modal.addEventListener('keydown', (e) => {
            if (e.key === 'Escape' && modal.dataset.dismissable === '1') {
                e.preventDefault();
                e.stopPropagation();
                modal.dataset.dismissable = '';
                finishOnboarding(true);
            }
        });
    }
    // Run after the initial config load settles.
    setTimeout(checkOnboarding, 900);
});
// ==========================================================================
// Browser context menu (right-click on file/folder rows)
// ==========================================================================
let __browserContextMenuEl = null;

function closeBrowserContextMenu() {
    if (__browserContextMenuEl) {
        __browserContextMenuEl.remove();
        __browserContextMenuEl = null;
    }
}

function showBrowserContextMenu(ev, entry) {
    ev.preventDefault();
    ev.stopPropagation();
    closeBrowserContextMenu();

    const menu = document.createElement('div');
    menu.className = 'browser-context-menu';
    menu.style.cssText = [
        'position:fixed', `left:${ev.clientX}px`, `top:${ev.clientY}px`,
        'z-index:10000', 'background:#1e2430', 'border:1px solid #3d4757',
        'border-radius:8px', 'padding:4px', 'min-width:180px',
        'box-shadow:0 8px 24px rgba(0,0,0,.5)', 'font-size:0.82rem'
    ].join(';');

    const addItem = (label, fn, opts = {}) => {
        const b = document.createElement('button');
        b.type = 'button';
        b.textContent = label;
        b.style.cssText = [
            'display:block', 'width:100%', 'text-align:left', 'padding:6px 10px',
            'background:transparent', 'border:0', 'border-radius:5px', 'cursor:pointer',
            'color:' + (opts.danger ? '#f85149' : '#e6edf3')
        ].join(';');
        b.addEventListener('mouseenter', () => { b.style.background = '#2b3340'; });
        b.addEventListener('mouseleave', () => { b.style.background = 'transparent'; });
        b.addEventListener('click', async () => { closeBrowserContextMenu(); await fn(); });
        menu.appendChild(b);
    };

    const sep = () => {
        const d = document.createElement('div');
        d.style.cssText = 'height:1px;background:#3d4757;margin:4px 2px;';
        menu.appendChild(d);
    };

    if (entry.is_dir) {
        addItem('Open', () => fetchDirectory(entry.path, { pushHistory: true }));
        addItem('Use this folder', () => {
            selectedBrowsePath = entry.path;
            if (browserSelectedLabel) browserSelectedLabel.textContent = entry.path;
            // Highlight the chosen row instead of clearing the selection.
            document.querySelectorAll('.browser-entry-row').forEach(r => {
                r.classList.toggle('selected', r.dataset.path === entry.path);
            });
        });
        addItem('New folder inside', () => createFolderIn(entry.path));
        sep();
    }

    addItem('Copy path', async () => {
        try {
            await navigator.clipboard.writeText(entry.path);
            showToast('Path copied.', 'success');
        } catch (_) {
            showToast(entry.path, 'info', 8000);
        }
    });

    addItem('Rename...', async () => {
        const next = window.prompt('New name:', entry.name);
        if (!next || next.trim() === entry.name) return;
        try {
            const res = await fetch('/api/filesystem/rename', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ path: entry.path, new_name: next.trim() })
            });
            if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
            showToast('Renamed.', 'success');
            fetchDirectory(currentBrowsePath);
        } catch (e) {
            showToast('Rename failed: ' + e.message, 'error', 7000);
        }
    });

    sep();
    addItem('Delete', async () => {
        const ok = await uiConfirm(
            `Delete ${entry.is_dir ? 'folder' : 'file'} "${entry.name}"?${entry.is_dir ? ' This removes its contents.' : ''}`,
            { title: 'Delete', confirmText: 'Delete', danger: true }
        );
        if (!ok) return;
        try {
            const res = await fetch('/api/filesystem/delete', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ path: entry.path })
            });
            if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
            showToast('Deleted.', 'info');
            fetchDirectory(currentBrowsePath);
        } catch (e) {
            showToast('Delete failed: ' + e.message, 'error', 7000);
        }
    }, { danger: true });

    document.body.appendChild(menu);
    __browserContextMenuEl = menu;

    // Keep the menu on screen.
    const rect = menu.getBoundingClientRect();
    if (rect.right > window.innerWidth) menu.style.left = (window.innerWidth - rect.width - 8) + 'px';
    if (rect.bottom > window.innerHeight) menu.style.top = (window.innerHeight - rect.height - 8) + 'px';
}

function createFolderIn(parentPath) {
    const name = window.prompt('New folder name:');
    if (!name || !name.trim()) return;
    fetch('/api/filesystem/mkdir', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ parent_path: parentPath, name: name.trim() })
    }).then(async res => {
        if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
        showToast('Folder created.', 'success');
        fetchDirectory(currentBrowsePath);
    }).catch(e => showToast('Create folder failed: ' + e.message, 'error', 7000));
}

document.addEventListener('click', closeBrowserContextMenu);
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeBrowserContextMenu(); });
window.addEventListener('blur', closeBrowserContextMenu);

// ==========================================================================
// Delegated handlers for dynamically rendered markup
//
// Every button the app renders through `innerHTML` is wired here with data
// attributes instead of inline event-handler attributes. That is what lets the
// page ship with a `script-src 'self'` CSP (no 'unsafe-inline'): an injected
// inline script or handler attribute no longer executes. The attributes are
// per-element and cannot carry markup, so this is not a weaker wiring - it is
// the same call with the payload out of the HTML.
// ==========================================================================
document.addEventListener('click', (ev) => {
    const permBtn = ev.target.closest?.('[data-perm-id]');
    if (permBtn) {
        const permId = permBtn.getAttribute('data-perm-id');
        const approved = permBtn.getAttribute('data-perm-approved') === '1';
        const always = permBtn.getAttribute('data-perm-always') === '1';
        decidePermission(permId, approved, always);
        return;
    }
    const mcpBtn = ev.target.closest?.('[data-mcp-action]');
    if (mcpBtn) {
        const action = mcpBtn.getAttribute('data-mcp-action');
        const id = mcpBtn.getAttribute('data-mcp-id');
        if (action === 'test') testMcpServer(id);
        else if (action === 'edit') editMcpServer(id);
        else if (action === 'delete') deleteMcpServer(id);
        return;
    }
    const copyBtn = ev.target.closest?.('[data-copy-code]');
    if (copyBtn) {
        copyCodeBlock(copyBtn);
        return;
    }
    const pathLink = ev.target.closest?.('[data-copy-path]');
    if (pathLink) {
        copyPath(pathLink);
        return;
    }
    const presetBtn = ev.target.closest?.('button[data-preset]');
    if (presetBtn && typeof applyProviderPreset === 'function') {
        const preset = presetBtn.getAttribute('data-preset');
        if (preset) applyProviderPreset(preset);
    }
});

document.addEventListener('change', (ev) => {
    const toggle = ev.target.closest?.('[data-mcp-toggle]');
    if (toggle) {
        toggleMcpServer(toggle.getAttribute('data-mcp-toggle'), toggle.checked);
    }
});
// ==========================================================================
// Project context menu (right-click a project in the sidebar)
// ==========================================================================
function openProjectContextMenu(ev, project) {
    closeBrowserContextMenu();
    const menu = document.createElement('div');
    menu.className = 'browser-context-menu';
    menu.style.cssText = [
        'position:fixed', `left:${ev.clientX}px`, `top:${ev.clientY}px`,
        'z-index:10000', 'background:#1e2430', 'border:1px solid #3d4757',
        'border-radius:8px', 'padding:4px', 'min-width:190px',
        'box-shadow:0 8px 24px rgba(0,0,0,.5)', 'font-size:0.82rem'
    ].join(';');

    const addItem = (label, fn, opts = {}) => {
        const b = document.createElement('button');
        b.type = 'button';
        b.textContent = label;
        b.style.cssText = [
            'display:block', 'width:100%', 'text-align:left', 'padding:6px 10px',
            'background:transparent', 'border:0', 'border-radius:5px', 'cursor:pointer',
            'color:' + (opts.danger ? '#f85149' : '#e6edf3')
        ].join(';');
        b.addEventListener('mouseenter', () => { b.style.background = '#2b3340'; });
        b.addEventListener('mouseleave', () => { b.style.background = 'transparent'; });
        b.addEventListener('click', async () => { closeBrowserContextMenu(); await fn(); });
        menu.appendChild(b);
    };

    addItem('Project settings...', () => openProjectSettingsModal(project.id));
    addItem('New conversation', () => createSavedConversation(project.id));
    addItem('Open in file browser', () => openFileBrowser(project.path));
    addItem('Copy path', async () => {
        try {
            await navigator.clipboard.writeText(project.path);
            showToast('Path copied.', 'success');
        } catch (_) { showToast(project.path, 'info', 8000); }
    });

    if ((allProjects || []).length > 1) {
        const d = document.createElement('div');
        d.style.cssText = 'height:1px;background:#3d4757;margin:4px 2px;';
        menu.appendChild(d);
        addItem('Delete project', async () => {
            const ok = await uiConfirm(
                `Delete project "${project.name}"? Conversations are kept but moved out of this project.`,
                { title: 'Delete Project', confirmText: 'Delete Project', danger: true }
            );
            if (!ok) return;
            try {
                const res = await fetch(`/api/projects/${encodeURIComponent(project.id)}`, { method: 'DELETE' });
                if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
                showToast(`Deleted project "${project.name}".`, 'info');
                await fetchProjects();
                await fetchSessions();
            } catch (e) {
                showToast('Delete failed: ' + e.message, 'error', 7000);
            }
        }, { danger: true });
    }

    document.body.appendChild(menu);
    __browserContextMenuEl = menu;
    const rect = menu.getBoundingClientRect();
    if (rect.right > window.innerWidth) menu.style.left = (window.innerWidth - rect.width - 8) + 'px';
    if (rect.bottom > window.innerHeight) menu.style.top = (window.innerHeight - rect.height - 8) + 'px';
}

// ==========================================================================
// Subagent thinking effort: "step down" / "inherit" / fixed
// ==========================================================================
const REASONING_LEVEL_ORDER = ['none', 'minimal', 'low', 'medium', 'high'];

function normalizeReasoningLevel(raw) {
    const v = String(raw || '').trim().toLowerCase().replace(/-/g, '_');
    if (!v) return null;
    if (v === 'off' || v === 'disabled') return 'none';
    if (v === 'min' || v === 'minimum') return 'minimal';
    if (v === 'med' || v === 'mid' || v === 'middle' || v === 'normal' || v === 'default') return 'medium';
    if (v === 'max' || v === 'maximum' || v === 'extra_high' || v === 'xhigh' || v === 'ultra') return 'high';
    return REASONING_LEVEL_ORDER.includes(v) ? v : null;
}

function stepDownEffort(raw) {
    const level = normalizeReasoningLevel(raw);
    if (!level) return String(raw || '').trim();
    const idx = REASONING_LEVEL_ORDER.indexOf(level);
    return REASONING_LEVEL_ORDER[Math.max(0, idx - 1)];
}

/** Resolve the level a subagent will actually run at, for display. */
function resolveSubagentEffort(configured, orchestratorEffort) {
    const c = String(configured || '').trim().toLowerCase().replace(/-/g, '_');
    if (c === 'step_down' || c === 'one_lower' || c === 'lower' || c === 'downgrade' || c === 'reduce') {
        return stepDownEffort(orchestratorEffort);
    }
    if (c === 'inherit' || c === 'follow' || c === 'orchestrator' || c === 'same' || c === 'parent') {
        return String(orchestratorEffort || 'medium').trim() || 'medium';
    }
    if (!c) return 'medium';
    return String(configured).trim();
}

function syncSubagentEffortControls() {
    const sel = document.getElementById('orch-subagent-effort');
    const hint = document.getElementById('label-subagent-effort-resolved');
    if (!sel) return;
    const configured = draftOrchState.subagent_thinking_effort || 'step_down';
    // Only set the select when it holds a known option value.
    if ([...sel.options].some(o => o.value === configured)) {
        sel.value = configured;
    } else {
        sel.value = 'medium';
    }
    if (hint) {
        const orchEffort = draftOrchState.thinking_effort || (currentConfig && currentConfig.reasoning_effort) || 'high';
        const resolved = resolveSubagentEffort(configured, orchEffort);
        const label = formatReasoningLevelLabel ? formatReasoningLevelLabel(resolved) : resolved;
        if (sel.value === 'step_down') {
            hint.textContent = `Subagents run at ${label} — one level below the orchestrator (${formatReasoningLevelLabel ? formatReasoningLevelLabel(orchEffort) : orchEffort}).`;
        } else if (sel.value === 'inherit') {
            hint.textContent = `Subagents follow the orchestrator (${label}).`;
        } else {
            hint.textContent = `Subagents always run at ${label}.`;
        }
    }
}

document.addEventListener('DOMContentLoaded', () => {
    const sel = document.getElementById('orch-subagent-effort');
    if (sel) {
        sel.addEventListener('change', () => {
            draftOrchState.subagent_thinking_effort = sel.value;
            syncSubagentEffortControls();
        });
    }
});
