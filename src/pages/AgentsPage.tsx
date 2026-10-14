import { CopilotPluginStatus } from '../components/copilot/CopilotPluginStatus';
import type { MessageKey } from '../i18n/resources';
import { ClaudeCodeRoleRows } from '../components/claude-code/ClaudeCodeRoleRows';
import { ClaudeCodeModelInputs } from '../components/claude-code/ClaudeCodeModelInputs';
import { MessageNotice } from '../appNotice';
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ComponentType,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AlertTriangle,
  BadgeCheck,
  Bot,
  Check,
  ChevronDown,
  LoaderCircle,
  SlidersHorizontal,
  Trash2,
  X,
} from 'lucide-react';
import claudeIcon from '../assets/icons/claude.svg';
import codexIcon from '../assets/icons/codex.svg';
import copilotIcon from '../assets/icons/copilot.svg';
import deepseekIcon from '../assets/icons/deepseek.svg';
import hermesIcon from '../assets/icons/hermes.png';
import grokIcon from '../assets/icons/grok.svg';
import kimiIcon from '../assets/icons/kimi-light.svg';
import openclawIcon from '../assets/icons/openclaw.svg';
import opencodeIcon from '../assets/icons/opencode.svg';
import piIcon from '../assets/icons/pi-logo-on-light.svg';
import zcodeIcon from '../assets/icons/zcode.png';
import workbuddyIcon from '../assets/icons/workbuddy.png';
import antigravityIcon from '../assets/icons/antigravity.svg';
import {
  CLAUDE_CODE_DEFAULT_MAX_CONTEXT_TOKENS,
  followClaudeCodeContextWindow,
} from '../services/claudeCodeModels';
import {
  filterAgentModelsByAlias,
  findAgentModel,
  resolveAgentModelForAliasMode,
  resolveAgentModelSelection,
} from '../services/agentModelPicker';
import {
  resolveAgentModelMappingsDraftSourceForClient,
  sameAgentModel,
  sameAgentModelMappings,
} from '../services/agentConfigurationDraft';
import {
  parseAgentLaunchDirectoryHistory,
  rememberAgentLaunchDirectory,
  type AgentLaunchDirectoryHistory,
} from '../services/agentLaunchDirectoryHistory';
import {
  buildDeepSeekHarnessLaunchOptions,
  DEFAULT_DEEPSEEK_HARNESS_LAUNCH_DRAFT,
  type DeepSeekHarnessLaunchDraft,
  type DeepSeekHarnessLaunchMode,
  type DeepSeekHarnessLaunchOptions,
} from '../services/deepSeekHarnessLaunch';
import type { ModelOption } from '../services/modelService';
import { AgentModelPicker } from '../components/AgentModelPicker';
import { claudeDesktopAliasSuggestions, claudeDesktopDefaultAliases, createDefaultDesktopModels, desktopAliasNotice, desktopEntryValidation, desktopModelEntries, desktopModelId, desktopModelValidation, isClaudeDesktopModel, selectedDesktopModelEntries, suffixCollidingDesktopAliases, type ClaudeDesktopModelMapping } from '../services/claudeDesktopModels';
import { getCurrentLocale, translate, useI18n } from '../i18n';
import { CodexSessionsPanel } from './CodexSessionsPanel';
import { CodexModelCatalogDialog } from './CodexModelCatalogDialog';
import { DeepSeekHarnessCatalogDialog } from './DeepSeekHarnessCatalogDialog';
import { AgentConfigBackupDialog } from './AgentConfigBackupDialog';
import { AgentClientList } from './AgentClientList';
import { AgentConfigManagementPanel, AgentConfigurationFeedback, AgentRunControls } from './AgentControls';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import { useConfirmation } from '../components/ConfirmationDialog';
import { requestAppNavigation } from '../navigation';

const setupErrors: Record<string, MessageKey> = {
  COPILOT_SETUP_REQUIRED: 'agents.copilotSetup.required',
  COPILOT_SETUP_NOT_INSTALLED: 'agents.copilotSetup.notInstalled',
  COPILOT_SETUP_LAUNCH_FAILED: 'agents.copilotSetup.launchFailed',
  COPILOT_SETUP_DISABLED: 'agents.copilotSetup.disabled',
  COPILOT_SETUP_DIRTY: 'agents.copilotSetup.dirty',
  COPILOT_SETUP_UNSUPPORTED: 'agents.copilotSetup.unsupported',
  COPILOT_SETUP_PROFILE: 'agents.copilotSetup.profile',
  COPILOT_SETUP_FAILED: 'agents.copilotSetup.failed',
  COPILOT_SETUP_TIMEOUT: 'agents.copilotSetup.timeout',
};

function copilotSetupErrorKey(error: string): MessageKey | undefined {
  const code = error.match(/\bCOPILOT_SETUP_[A-Z_]+\b/)?.[0];
  return code ? setupErrors[code] : undefined;
}

type AgentClientId =
  | 'claude-code'
  | 'claude-desktop'
  | 'codex'
  | 'opencode'
  | 'openclaw'
  | 'hermes'
  | 'deepseek-harness'
  | 'zcode'
  | 'workbuddy'
  | 'antigravity-cli'
  | 'copilot-cli'
  | 'vscode-copilot'
  | 'kimi-code'
  | 'grok-build'
  | 'omp'
  | 'pi';

type ClaudeModelMappingClientId = 'claude-code' | 'claude-desktop';

type AgentModificationState = 'unconfigured' | 'applied' | 'invalid';

type AgentConfigStatus = {
  id: AgentClientId;
  name: string;
  supportedPlatform: boolean;
  installed: boolean;
  pluginInstalled: boolean;
  launchTargets: AgentLaunchTarget[];
  version: string | null;
  cliVersion: string | null;
  appVersion: string | null;
  pluginVersion: string | null;
  configExists: boolean;
  configValid: boolean;
  configured: boolean;
  connectionState: 'configured' | 'not-configured' | 'needs-update' | 'invalid';
  configurationSynchronized: boolean;
  currentModel: string | null;
  oauthConfiguration: boolean;
  codexNativeOauth: boolean;
  modificationEnabled: boolean;
  modificationState: AgentModificationState;
  backupAvailable: boolean;
  appliedModel: string | null;
  claudeCodeModelMappings: ClaudeModelMappings | null;
  claudeDesktopModelMappings: ClaudeModelMappings | null;
  warnings: string[];
  error: string | null;
};

type AgentLaunchTarget = {
  id: 'app' | 'cli';
  label: string;
  detail: string;
};

type DeepSeekHarnessProcessStatus = {
  running: boolean;
  pid: number | null;
  mode: string | null;
};

type AgentConfigActionResult = {
  outcome: 'applied' | 'default' | 'updated' | 'unchanged';
  enabled: boolean;
  model: string | null;
  changedFiles: string[];
  conflictFiles: string[];
};

type PiProviderUpdateStatus = {
  installedVersion: string | null;
  latestVersion: string | null;
  updateAvailable: boolean;
};

type OAuthLoginRequiredAction = 'enable' | 'apply' | 'launch';

type ClaudeModelMappings = {
  desktopModels?: ClaudeDesktopModelMapping[];
  opus: string;
  sonnet: string;
  haiku: string;
  fable: string;
  fable1m: boolean;
  opus1m: boolean;
  sonnet1m: boolean;
  haiku1m: boolean;
  maxContextTokens: number;
  autoCompactPct: number;
  disableAutoCompact: boolean;
  manageDefaultModel: boolean;
  manageSubagentModel: boolean;
  startupModel: string;
  subagentModel: string;
};

type AgentFormValues = {
  model: string;
  oauthConfiguration: boolean;
  mappings: ClaudeModelMappings | null;
};

const sameAgentFormValues = (left: AgentFormValues, right: AgentFormValues) => (
  sameAgentModel(left.model, right.model)
  && left.oauthConfiguration === right.oauthConfiguration
  && (left.mappings && right.mappings
    ? sameAgentModelMappings(left.mappings, right.mappings)
    : left.mappings === right.mappings)
);

const CODEX_OAUTH_LOGIN_REQUIRED_ERROR = 'CODEX_OAUTH_LOGIN_REQUIRED';
const DEFAULT_CLAUDE_AUTO_COMPACT_PCT = 90;

const createClaudeModelMappings = (): ClaudeModelMappings => ({
  opus: '',
  sonnet: '',
  haiku: '',
  fable: '',
  fable1m: false,
  opus1m: false,
  sonnet1m: false,
  haiku1m: false,
  maxContextTokens: CLAUDE_CODE_DEFAULT_MAX_CONTEXT_TOKENS,
  autoCompactPct: DEFAULT_CLAUDE_AUTO_COMPACT_PCT,
  disableAutoCompact: false,
  manageDefaultModel: true,
  manageSubagentModel: false,
  startupModel: 'opus',
  subagentModel: '',
});

const createClaudeModelMappingsByClient = (): Record<
  ClaudeModelMappingClientId,
  ClaudeModelMappings
> => ({
  'claude-code': createClaudeModelMappings(),
  'claude-desktop': { ...createClaudeModelMappings(), desktopModels: createDefaultDesktopModels() },
});

const createClaudeBooleanByClient = (): Record<ClaudeModelMappingClientId, boolean> => ({
  'claude-code': false,
  'claude-desktop': false,
});

let claudeModelMappingsDraftCache = createClaudeModelMappingsByClient();
let claudeCustomMappingCache = createClaudeBooleanByClient();
const claudeModelMappingsDirtyCache = createClaudeBooleanByClient();
let codexOauthConfigurationDraftCache: boolean | null = null;
let agentFormEditBaselineCache: Partial<Record<AgentClientId, AgentFormValues>> = {};

const claudeMappingRoles = [
  { key: 'fable', contextKey: 'fable1m', labelKey: 'agents.claudeDesktopMapping.fable' },
  {
    key: 'opus',
    contextKey: 'opus1m',
    labelKey: 'agents.claudeDesktopMapping.opus',
  },
  {
    key: 'sonnet',
    contextKey: 'sonnet1m',
    labelKey: 'agents.claudeDesktopMapping.sonnet',
  },
  {
    key: 'haiku',
    contextKey: 'haiku1m',
    labelKey: 'agents.claudeDesktopMapping.haiku',
  },
] as const;

type AgentDefinition = {
  id: AgentClientId;
  name: string;
  icon?: string;
  Icon?: ComponentType<{ size?: number; 'aria-hidden'?: boolean }>;
  descriptionKey: 'agents.description.copilotCli' | 'agents.description.vscodeCopilot' | 'agents.description.claudeCode' | 'agents.description.claudeDesktop' | 'agents.description.codex' | 'agents.description.opencode' | 'agents.description.openclaw' | 'agents.description.hermes' | 'agents.description.deepseekHarness' | 'agents.description.zcode' | 'agents.description.workbuddy' | 'agents.description.antigravityCli' | 'agents.description.kimiCode' | 'agents.description.grokBuild' | 'agents.description.omp' | 'agents.description.pi';
};

type AgentSubpageId = 'core' | 'management' | 'sessions';

type AgentSubpageDefinition = {
  id: AgentSubpageId;
  labelKey: 'agents.tabs.core' | 'agents.tabs.management' | 'agents.tabs.sessions';
  clients?: readonly AgentClientId[];
};

const agentDefinitions: AgentDefinition[] = [
  {
    id: 'claude-code',
    name: 'Claude Code',
    icon: claudeIcon,
    descriptionKey: 'agents.description.claudeCode',
  },
  {
    id: 'claude-desktop',
    name: 'Claude Desktop',
    icon: claudeIcon,
    descriptionKey: 'agents.description.claudeDesktop',
  },
  {
    id: 'codex',
    name: 'Codex',
    icon: codexIcon,
    descriptionKey: 'agents.description.codex',
  },
  {
    id: 'deepseek-harness',
    name: 'DeepSeek Harness',
    icon: deepseekIcon,
    descriptionKey: 'agents.description.deepseekHarness',
  },
  {
    id: 'opencode',
    name: 'OpenCode',
    icon: opencodeIcon,
    descriptionKey: 'agents.description.opencode',
  },
  {
    id: 'omp',
    name: 'Oh My Pi',
    icon: piIcon,
    descriptionKey: 'agents.description.omp',
  },
  {
    id: 'pi',
    name: 'Pi',
    icon: piIcon,
    descriptionKey: 'agents.description.pi',
  },
  {
    id: 'grok-build',
    name: 'Grok Build',
    icon: grokIcon,
    descriptionKey: 'agents.description.grokBuild',
  },
  { id: 'copilot-cli', name: 'Copilot CLI', Icon: Bot, descriptionKey: 'agents.description.copilotCli' },
  { id: 'vscode-copilot', name: 'VSCode Copilot', icon: copilotIcon, descriptionKey: 'agents.description.vscodeCopilot' },
  { id: 'antigravity-cli', name: 'Antigravity CLI', icon: antigravityIcon, descriptionKey: 'agents.description.antigravityCli' },
  {
    id: 'workbuddy',
    name: 'WorkBuddy',
    icon: workbuddyIcon,
    descriptionKey: 'agents.description.workbuddy',
  },
  {
    id: 'zcode',
    name: 'ZCode',
    icon: zcodeIcon,
    descriptionKey: 'agents.description.zcode',
  },
  {
    id: 'kimi-code',
    name: 'Kimi Code',
    icon: kimiIcon,
    descriptionKey: 'agents.description.kimiCode',
  },
  {
    id: 'openclaw',
    name: 'OpenClaw',
    icon: openclawIcon,
    descriptionKey: 'agents.description.openclaw',
  },
  {
    id: 'hermes',
    name: 'Hermes Agent',
    icon: hermesIcon,
    descriptionKey: 'agents.description.hermes',
  },
];

const agentSubpages: AgentSubpageDefinition[] = [
  {
    id: 'core',
    labelKey: 'agents.tabs.core',
  },
  {
    id: 'management',
    labelKey: 'agents.tabs.management',
    clients: [
      'claude-code',
      'claude-desktop',
      'codex',
      'opencode',
      'openclaw',
      'hermes',
      'deepseek-harness',
      'zcode',
      'workbuddy',
      'antigravity-cli',
      'copilot-cli',
      'vscode-copilot',
      'kimi-code',
      'omp',
      'grok-build',
      'pi',
    ],
  },
  {
    id: 'sessions',
    labelKey: 'agents.tabs.sessions',
    clients: ['codex'],
  },
];

const DEFAULT_AGENT_SUBPAGE: AgentSubpageId = 'core';

type AgentViewState = {
  subpage: AgentSubpageId;
  connectionHelpOpen: boolean;
  configurationError: string;
  configurationNotice: string;
  clearNotice: string;
  launchError: string;
};

const DEFAULT_AGENT_VIEW_STATE: AgentViewState = {
  subpage: DEFAULT_AGENT_SUBPAGE,
  connectionHelpOpen: false,
  configurationError: '',
  configurationNotice: '',
  clearNotice: '',
  launchError: '',
};

let agentViewStateCache: Record<'full' | 'embedded', Partial<Record<AgentClientId, AgentViewState>>> = {
  full: {},
  embedded: {},
};

const AGENT_MODEL_SELECTIONS_KEY = 'cpa-gui.agent-model-selections.v1';
// Keep the last successful data across page visits; revalidate it in the background.
let agentStatusesCache: AgentConfigStatus[] | null = null;
const agentModelsCache: Partial<Record<AgentClientId, ModelOption[]>> = {};
// A Codex daemon restart outlives page visits; a remounted page waits on it instead of starting another.
let codexCliServiceRestart: Promise<void> | null = null;
const AGENT_SELECTED_CLIENT_KEY = 'cpa-gui.agent-selected-client.v1';
const AGENT_LAUNCH_DIRECTORY_HISTORY_KEY = 'cpa-gui.agent-launch-directory-history.v1';
const AGENT_EXECUTABLE_PATHS_KEY = 'cpa-gui.agent-executable-paths.v1';
type AgentExecutablePaths = Partial<Record<AgentClientId | `${AgentClientId}:app`, string>>;
const readAgentExecutablePaths = (): AgentExecutablePaths => {
  if (typeof window === 'undefined') return {};
  try {
    const parsed = JSON.parse(window.localStorage.getItem(AGENT_EXECUTABLE_PATHS_KEY) || '{}') as Record<string, unknown>;
    return agentDefinitions.reduce<AgentExecutablePaths>((result, agent) => {
      for (const key of [agent.id, `${agent.id}:app`] as const) {
        const value = parsed[key];
        if (typeof value === 'string' && value.trim()) result[key] = value.trim();
      }
      return result;
    }, {});
  } catch { return {}; }
};
const writeAgentExecutablePaths = (paths: AgentExecutablePaths) => {
  try { window.localStorage.setItem(AGENT_EXECUTABLE_PATHS_KEY, JSON.stringify(paths)); } catch { /* ignore storage failures */ }
};

const readSelectedAgentClient = (): AgentClientId => {
  const fallback = agentDefinitions[0].id;
  if (typeof window === 'undefined') return fallback;
  try {
    const saved = window.localStorage.getItem(AGENT_SELECTED_CLIENT_KEY);
    return agentDefinitions.some((agent) => agent.id === saved)
      ? (saved as AgentClientId)
      : fallback;
  } catch {
    return fallback;
  }
};

const writeSelectedAgentClient = (client: AgentClientId) => {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(AGENT_SELECTED_CLIENT_KEY, client);
  } catch {
  }
};

const readAgentModelSelections = (): Partial<Record<AgentClientId, string>> => {
  if (typeof window === 'undefined') return {};
  try {
    const payload = window.localStorage.getItem(AGENT_MODEL_SELECTIONS_KEY);
    if (!payload) return {};
    const parsed = JSON.parse(payload) as Record<string, unknown>;
    return agentDefinitions.reduce<Partial<Record<AgentClientId, string>>>((result, agent) => {
      const value = parsed[agent.id];
      if (typeof value === 'string' && value.trim()) result[agent.id] = value.trim();
      return result;
    }, {});
  } catch {
    return {};
  }
};

const writeAgentModelSelections = (
  selections: Partial<Record<AgentClientId, string>>,
) => {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(AGENT_MODEL_SELECTIONS_KEY, JSON.stringify(selections));
  } catch {
  }
};

const readAgentLaunchDirectoryHistory = (): AgentLaunchDirectoryHistory => {
  if (typeof window === 'undefined') return {};
  try {
    return parseAgentLaunchDirectoryHistory(
      window.localStorage.getItem(AGENT_LAUNCH_DIRECTORY_HISTORY_KEY),
      agentDefinitions.map((agent) => agent.id),
    );
  } catch {
    return {};
  }
};

const writeAgentLaunchDirectoryHistory = (history: AgentLaunchDirectoryHistory) => {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(AGENT_LAUNCH_DIRECTORY_HISTORY_KEY, JSON.stringify(history));
  } catch {
  }
};

function AgentMark({ definition, size = 26 }: { definition: AgentDefinition; size?: number }) {
  if (definition.icon) {
    return <img src={definition.icon} alt="" className="provider-logo" />;
  }
  const Icon = definition.Icon ?? Bot;
  return <Icon size={size} aria-hidden />;
}

const listStatusText = (status: AgentConfigStatus | undefined) => {
  const locale = getCurrentLocale();
  if (!status) return translate(locale, 'agents.list.detecting');
  if (!status.supportedPlatform) return translate(locale, 'agents.list.unsupported');
  if (!status.installed) return translate(locale, 'agents.list.notInstalled');
  if (status.id === 'pi') {
    return status.pluginInstalled
      ? translate(locale, 'agents.list.piInstalled')
      : translate(locale, 'agents.list.pluginNotInstalled');
  }
  if (status.modificationState === 'invalid') return translate(locale, 'agents.status.invalid');
  if (status.id === 'codex' && status.codexNativeOauth) return translate(locale, 'agents.nativeOAuth.status');
  if (status.modificationState === 'applied') return translate(locale, 'agents.list.modified', { model: status.appliedModel ?? '—' });
  return status.version
    ? translate(locale, 'agents.list.installedVersion', { version: status.version })
    : translate(locale, 'agents.list.installed');
};

function ClaudeDesktopHelpDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, []);
  return (
    <dialog ref={dialogRef} className="config-dialog agent-desktop-help-dialog" aria-labelledby={titleId}
      onCancel={(event) => { event.preventDefault(); onClose(); }}
      onMouseDown={(event) => {
        if (event.target !== event.currentTarget) return;
        const rect = event.currentTarget.getBoundingClientRect();
        if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose();
      }}>
      <div className="config-dialog-heading">
        <h2 id={titleId}>{t('agents.claudeDesktopMapping.help')}</h2>
        <button type="button" className="icon-button quiet" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
      </div>
      <ol className="agent-desktop-help-list">
        <li>{t('agents.claudeDesktopMapping.description')}</li>
        <li>{t('agents.claudeDesktopMapping.collisionHint')}</li>
        <li>{t('agents.claudeDesktopMapping.idHint')}</li>
      </ol>
      <div className="agent-desktop-help-actions">
        <button type="button" className="primary-button" onClick={onClose}>{t('common.close')}</button>
      </div>
    </dialog>
  );
}

type AgentsPageProps = {
  embedded?: boolean;
  onConfigurationApplied?: () => void;
};

export function AgentsPage({ embedded = false, onConfigurationApplied }: AgentsPageProps = {}) {
  const { t } = useI18n();
  const [selected, setSelected] = useState<AgentClientId>(readSelectedAgentClient);
  const [viewStateByClient, setViewStateByClient] = useState(() => agentViewStateCache);
  const viewMode = embedded ? 'embedded' : 'full';
  const viewState = viewStateByClient[viewMode][selected] ?? DEFAULT_AGENT_VIEW_STATE;
  const activeSubpage = viewState.subpage === 'sessions' && (embedded || selected !== 'codex')
    ? DEFAULT_AGENT_SUBPAGE : viewState.subpage;
  const updateViewStateForClient = (client: AgentClientId, patch: Partial<AgentViewState>) => {
    setViewStateByClient((current) => {
      const next = {
        ...current,
        [viewMode]: {
          ...current[viewMode],
          [client]: { ...(current[viewMode][client] ?? DEFAULT_AGENT_VIEW_STATE), ...patch },
        },
      };
      agentViewStateCache = next;
      return next;
    });
  };
  const updateViewState = (patch: Partial<AgentViewState>) => updateViewStateForClient(selected, patch);
  const setActiveSubpage = (subpage: AgentSubpageId) => {
    if (subpage !== activeSubpage) requestAppNavigation(() => updateViewState({ subpage }));
  };
  const { connectionHelpOpen, configurationError, configurationNotice, clearNotice, launchError } = viewState;
  const setConfigurationError = (configurationError: string) => updateViewState({ configurationError });
  const setConfigurationNotice = (configurationNotice: string) => updateViewState({ configurationNotice });
  const setClearNotice = (clearNotice: string) => updateViewState({ clearNotice });
  const setLaunchError = (launchError: string) => updateViewState({ launchError });
  const [statuses, setStatuses] = useState<AgentConfigStatus[]>(() => agentStatusesCache ?? []);
  const [executablePaths, setExecutablePaths] = useState(() => readAgentExecutablePaths());
  const [modelData, setModelData] = useState(() => ({
    client: selected, models: agentModelsCache[selected] ?? [],
  }));
  const models = modelData.client === selected ? modelData.models : agentModelsCache[selected] ?? [];
  const [modelByClient, setModelByClient] = useState<Partial<Record<AgentClientId, string>>>(
    readAgentModelSelections,
  );
  const [formEditBaselineByClient, setFormEditBaselineByClient] = useState(
    () => ({ ...agentFormEditBaselineCache }),
  );
  const [claudeModelMappingsDraftByClient, setClaudeModelMappingsDraftByClientState] = useState(
    () => ({
      'claude-code': { ...claudeModelMappingsDraftCache['claude-code'] },
      'claude-desktop': { ...claudeModelMappingsDraftCache['claude-desktop'] },
    }),
  );
  const [claudeCustomMappingByClient, setClaudeCustomMappingByClientState] = useState(
    () => ({ ...claudeCustomMappingCache }),
  );
  const [loading, setLoading] = useState(() => agentStatusesCache === null);
  const [modelLoading, setModelLoading] = useState(false);
  const [busyAction, setBusyAction] = useState<
    'backup' | 'apply' | 'close-config' | 'default' | 'clear' | 'install-pi' | 'update-pi' | 'repair-pi' | 'uninstall-pi' | 'oauth-check' | 'native-oauth' | 'directory' | 'launch' | 'launch-cli' | 'launch-app' | 'restart-app' | 'stop-deepseek' | 'restart-deepseek' | null
  >(null);
  const busy = busyAction !== null;
  // Codex drains running turns before restarting its daemon, which can take about a minute, so the
  // restart only locks its own controls instead of the page-wide busy state.
  const [cliServiceRestarting, setCliServiceRestarting] = useState(() => codexCliServiceRestart !== null);
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const [detectionError, setDetectionError] = useState('');
  const [modelError, setModelError] = useState('');
  const [modelSelectionError, setModelSelectionError] = useState('');
  const [backupsOpen, setBackupsOpen] = useState(false);
  const [defaultError, setDefaultError] = useState('');
  const [templatePreview, setTemplatePreview] = useState<{ revision: string; files: string[] } | null>(null);
  const [defaultConfirmOpen, setDefaultConfirmOpen] = useState(false);
  const [clearError, setClearError] = useState('');
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [launchDirectoryDialogOpen, setLaunchDirectoryDialogOpen] = useState(false);
  const [codexCatalogDialogOpen, setCodexCatalogDialogOpen] = useState(false);
  const [harnessCatalogDialogOpen, setHarnessCatalogDialogOpen] = useState(false);
  const [desktopHelpOpen, setDesktopHelpOpen] = useState(false);
  const [launchDirectory, setLaunchDirectory] = useState('');
  const [launchDirectoryTarget, setLaunchDirectoryTarget] = useState<AgentLaunchTarget | null>(null);
  const [launchDirectoryError, setLaunchDirectoryError] = useState('');
  const [deepSeekHarnessLaunchDraft, setDeepSeekHarnessLaunchDraft] = useState<DeepSeekHarnessLaunchDraft>(
    () => ({ ...DEFAULT_DEEPSEEK_HARNESS_LAUNCH_DRAFT }),
  );
  const [deepSeekHarnessProcessStatus, setDeepSeekHarnessProcessStatus] = useState<DeepSeekHarnessProcessStatus>({
    running: false,
    pid: null,
    mode: null,
  });
  const [launchDirectoryHistory, setLaunchDirectoryHistory] = useState(
    readAgentLaunchDirectoryHistory,
  );
  const [oauthLoginRequiredAction, setOauthLoginRequiredAction] = useState<OAuthLoginRequiredAction | null>(null);
  const [oauthConfigurationDraft, setOauthConfigurationDraftState] = useState<boolean | null>(
    () => codexOauthConfigurationDraftCache,
  );
  const [piProviderUpdateStatus, setPiProviderUpdateStatus] = useState<PiProviderUpdateStatus | null>(null);
  const modelRequestRef = useRef(0);
  const statusRequestRef = useRef(0);
  const piUpdateRequestRef = useRef(0);
  const claudeModelMappingsDirtyRef = useRef(claudeModelMappingsDirtyCache);
  const launchDirectoryDialogRef = useDialogFocusTrap<HTMLElement>({
    active: launchDirectoryDialogOpen,
    onEscape: busy
      ? undefined
      : () => {
          setLaunchDirectoryDialogOpen(false);
          setLaunchDirectoryTarget(null);
        },
    preventEscape: busy,
  });
  const defaultDialogRef = useDialogFocusTrap<HTMLElement>({
    active: defaultConfirmOpen,
    onEscape: busy ? undefined : () => setDefaultConfirmOpen(false),
    preventEscape: busy,
  });
  const clearDialogRef = useDialogFocusTrap<HTMLElement>({
    active: clearConfirmOpen,
    onEscape: busy ? undefined : () => setClearConfirmOpen(false),
    preventEscape: busy,
  });
  const oauthRequiredDialogRef = useDialogFocusTrap<HTMLElement>({
    active: Boolean(oauthLoginRequiredAction),
    onEscape: () => setOauthLoginRequiredAction(null),
  });

  const setClaudeModelMappingsDraftByClient = useCallback((
    update: (
      current: Record<ClaudeModelMappingClientId, ClaudeModelMappings>,
    ) => Record<ClaudeModelMappingClientId, ClaudeModelMappings>,
  ) => {
    setClaudeModelMappingsDraftByClientState((current) => {
      const next = update(current);
      claudeModelMappingsDraftCache = next;
      return next;
    });
  }, []);

  const setClaudeCustomMappingByClient = useCallback((
    update: (
      current: Record<ClaudeModelMappingClientId, boolean>,
    ) => Record<ClaudeModelMappingClientId, boolean>,
  ) => {
    setClaudeCustomMappingByClientState((current) => {
      const next = update(current);
      claudeCustomMappingCache = next;
      return next;
    });
  }, []);

  const setOauthConfigurationDraft = useCallback((value: boolean | null) => {
    codexOauthConfigurationDraftCache = value;
    setOauthConfigurationDraftState(value);
  }, []);

  const loadStatuses = useCallback(async (forceRefresh = false) => {
    const requestId = ++statusRequestRef.current;
    const command = forceRefresh
      ? 'refresh_agent_config_statuses'
      : 'get_agent_config_statuses';
    const nextStatuses = await invoke<AgentConfigStatus[]>(command, { executableOverrides: executablePaths });
    if (requestId !== statusRequestRef.current) return;
    setStatuses(nextStatuses);
  }, [executablePaths]);

  useEffect(() => {
    if (statuses.length) agentStatusesCache = statuses;
  }, [statuses]);

  useEffect(() => () => {
    // Responses from a previous visit must not overwrite the shared cache.
    modelRequestRef.current += 1;
    statusRequestRef.current += 1;
  }, []);

  const loadDeepSeekHarnessProcessStatus = useCallback(async () => {
    const status = await invoke<DeepSeekHarnessProcessStatus>('get_deepseek_harness_process_status');
    setDeepSeekHarnessProcessStatus(status);
  }, []);

  const loadModels = useCallback(async (client: AgentClientId, preferredModel = '', background = false) => {
    const requestId = modelRequestRef.current + 1;
    modelRequestRef.current = requestId;
    setModelLoading(!background || agentModelsCache[client] === undefined);
    setModelError('');
    try {
      const nextModels = await invoke<ModelOption[]>('get_agent_models', { client });
      if (modelRequestRef.current !== requestId) return;
      agentModelsCache[client] = nextModels;
      setModelData({ client, models: nextModels });
      setModelSelectionError('');
      setModelByClient((current) => {
        const next = {
          ...current,
          [client]: resolveAgentModelSelection(nextModels, current[client] || preferredModel),
        };
        writeAgentModelSelections(next);
        return next;
      });
    } catch (requestError) {
      if (modelRequestRef.current === requestId) setModelError(String(requestError));
    } finally {
      if (modelRequestRef.current === requestId) setModelLoading(false);
    }
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    setDetectionError('');
    try {
      await loadStatuses(true);
    } catch (requestError) {
      setDetectionError(String(requestError));
    } finally {
      setLoading(false);
    }
  }, [loadStatuses]);

  useEffect(() => {
    setLoading(agentStatusesCache === null);
    setDetectionError('');
    void loadStatuses()
      .catch((requestError) => setDetectionError(String(requestError)))
      .finally(() => setLoading(false));
  }, [loadStatuses]);

  useEffect(() => {
    if (loading) return;
    const status = statuses.find((status) => status.id === selected);
    const preferredModel = status?.currentModel ?? '';
    void loadModels(selected, preferredModel, true);
  }, [loadModels, loading, selected]);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | null = null;
    void listen('config-files-changed', () => {
      if (disposed) return;
      setDetectionError('');
      void loadStatuses().catch((requestError) => {
        if (!disposed) setDetectionError(String(requestError));
      });
      void loadModels(selected, '', true);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [loadModels, loadStatuses, selected]);

  useEffect(() => {
    if (selected !== 'deepseek-harness') return undefined;
    let disposed = false;
    const refreshProcessStatus = () => {
      void loadDeepSeekHarnessProcessStatus().catch((requestError) => {
        if (!disposed) setLaunchError(String(requestError));
      });
    };
    refreshProcessStatus();
    const timer = window.setInterval(refreshProcessStatus, 2000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [loadDeepSeekHarnessProcessStatus, selected]);

  useEffect(() => {
    writeSelectedAgentClient(selected);
  }, [selected]);

  useEffect(() => {
    setModelSelectionError('');
    setBackupsOpen(false);
    setDefaultError('');
    setDefaultConfirmOpen(false);
    setClearError('');
    setClearConfirmOpen(false);
    setCodexCatalogDialogOpen(false);
    setDesktopHelpOpen(false);
    setLaunchDirectoryDialogOpen(false);
    setLaunchDirectoryTarget(null);
    setLaunchDirectoryError('');
    setOauthLoginRequiredAction(null);
  }, [selected]);

  const activeDefinition = agentDefinitions.find((agent) => agent.id === selected)
    ?? agentDefinitions[0];
  const activeStatus = statuses.find((status) => status.id === selected) ?? null;
  const nativeOauth = selected === 'codex' && Boolean(activeStatus?.codexNativeOauth);
  const oauthConfiguration = oauthConfigurationDraft
    ?? activeStatus?.oauthConfiguration
    ?? false;
  const savedSelectedModel = modelByClient[selected] ?? '';
  const selectedModelOption = findAgentModel(models, savedSelectedModel);
  const selectedModel = selectedModelOption?.name ?? '';
  const missingModelHint = !modelLoading && savedSelectedModel && !selectedModelOption
    ? t('agents.error.selectionGone') : '';
  const isPiClient = selected === 'pi';
  const isDeepSeekHarnessClient = selected === 'deepseek-harness';
  const hasIndependentCliAndApp = selected === 'codex' || selected === 'opencode' || isDeepSeekHarnessClient;
  const isClaudeModelMappingClient = selected === 'claude-code' || selected === 'claude-desktop';
  const claudeModelMappingsDraft = isClaudeModelMappingClient
    ? claudeModelMappingsDraftByClient[selected]
    : createClaudeModelMappings();
  const claudeCustomMapping = isClaudeModelMappingClient
    ? claudeCustomMappingByClient[selected]
    : false;
  const claudeMappingModels = useMemo(
    () => filterAgentModelsByAlias(models, claudeCustomMapping),
    [claudeCustomMapping, models],
  );
  const desktopEntries = claudeModelMappingsDraft.desktopModels ?? desktopModelEntries(claudeModelMappingsDraft);
  const appliedDesktopEntries = activeStatus?.claudeDesktopModelMappings
    ? desktopModelEntries(activeStatus.claudeDesktopModelMappings) : [];
  const resolvedDesktopEntries = suffixCollidingDesktopAliases(desktopEntries, models, appliedDesktopEntries);
  const desktopValidation = desktopModelValidation(resolvedDesktopEntries, models, appliedDesktopEntries);

  const loadPiProviderUpdateStatus = useCallback(async () => {
    const requestId = piUpdateRequestRef.current + 1;
    piUpdateRequestRef.current = requestId;
    try {
      const nextStatus = await invoke<PiProviderUpdateStatus>('check_pi_provider_update');
      if (piUpdateRequestRef.current === requestId) setPiProviderUpdateStatus(nextStatus);
    } catch {
      if (piUpdateRequestRef.current === requestId) setPiProviderUpdateStatus(null);
    }
  }, []);

  useEffect(() => {
    if (!isPiClient || !activeStatus?.pluginInstalled || !activeStatus.pluginVersion) {
      piUpdateRequestRef.current += 1;
      setPiProviderUpdateStatus(null);
      return;
    }
    void loadPiProviderUpdateStatus();
  }, [activeStatus?.pluginInstalled, activeStatus?.pluginVersion, isPiClient, loadPiProviderUpdateStatus]);

  const piPluginUpdateAvailable = Boolean(
    piProviderUpdateStatus?.updateAvailable
      && piProviderUpdateStatus.installedVersion === activeStatus?.pluginVersion
      && piProviderUpdateStatus.latestVersion,
  );
  const piPluginUpdateTitle = piPluginUpdateAvailable
    ? t('agents.pi.updateAvailable', { version: piProviderUpdateStatus?.latestVersion ?? '' })
    : activeStatus?.pluginVersion ?? undefined;

  useEffect(() => {
    if (!isClaudeModelMappingClient) return;
    const appliedMappings = selected === 'claude-code'
      ? activeStatus?.claudeCodeModelMappings
      : activeStatus?.claudeDesktopModelMappings;
    const dirty = claudeModelMappingsDirtyRef.current[selected];
    if (!dirty && selected === 'claude-code' && models.length > 0) {
      const appliedModels = appliedMappings
        ? claudeMappingRoles
            .map((role) => findAgentModel(models, appliedMappings[role.key]))
            .filter((model): model is ModelOption => model !== null)
        : [];
      setClaudeCustomMappingByClient((current) => ({
        ...current,
        [selected]: appliedModels.length === claudeMappingRoles.length
          && appliedModels.every((model) => Boolean(model.isAlias)),
      }));
    }
    if (!dirty && selected === 'claude-desktop' && models.length > 0 && appliedMappings) {
      const appliedEntries = desktopModelEntries(appliedMappings).filter((entry) => entry.model.trim());
      const appliedModels = appliedEntries
        .map((entry) => findAgentModel(models, entry.model))
        .filter((model): model is ModelOption => model !== null);
      if (appliedModels.length > 0 && appliedModels.length === appliedEntries.length) {
        setClaudeCustomMappingByClient((current) => ({
          ...current,
          [selected]: appliedModels.every((model) => Boolean(model.isAlias)),
        }));
      }
    }
    setClaudeModelMappingsDraftByClient((current) => {
      const currentClientDraft = current[selected];
      const source = resolveAgentModelMappingsDraftSourceForClient(
        current,
        selected,
        appliedMappings,
        createClaudeModelMappings(),
        dirty,
      );
      const next: ClaudeModelMappings = {
        ...(selected === 'claude-desktop' ? { desktopModels: dirty && source.desktopModels
          ? source.desktopModels : appliedMappings ? desktopModelEntries(source) : createDefaultDesktopModels() } : {}),
        opus: findAgentModel(models, source.opus)?.name ?? '',
        sonnet: findAgentModel(models, source.sonnet)?.name ?? '',
        haiku: findAgentModel(models, source.haiku)?.name ?? '',
        opus1m: Boolean(source.opus1m),
        sonnet1m: Boolean(source.sonnet1m),
        haiku1m: Boolean(source.haiku1m),
        fable: findAgentModel(models, source.fable)?.name ?? '',
        fable1m: Boolean(source.fable1m),
        maxContextTokens: source.maxContextTokens ?? CLAUDE_CODE_DEFAULT_MAX_CONTEXT_TOKENS,
        autoCompactPct: source.autoCompactPct ?? DEFAULT_CLAUDE_AUTO_COMPACT_PCT,
        disableAutoCompact: Boolean(source.disableAutoCompact),
        manageDefaultModel: source.manageDefaultModel !== false,
        manageSubagentModel: source.manageSubagentModel !== false,
        startupModel: source.startupModel?.trim() || 'opus',
        subagentModel: source.subagentModel ?? '',
      };
      return sameAgentModelMappings(currentClientDraft, next)
        ? current
        : { ...current, [selected]: next };
    });
  }, [
    activeStatus?.claudeCodeModelMappings,
    activeStatus?.claudeDesktopModelMappings,
    isClaudeModelMappingClient,
    models,
    selected,
  ]);

  const connectionState = activeStatus?.connectionState ?? 'invalid';
  const appliedModel = activeStatus?.currentModel ?? activeStatus?.appliedModel ?? '';
  const canCloseConfiguration = !isPiClient && !nativeOauth
    && (connectionState === 'configured' || (selected !== 'codex' && connectionState === 'needs-update'));
  const configurationActionLabel = t(nativeOauth ? 'agents.modify.update'
    : connectionState === 'not-configured' ? selected === 'codex' ? 'agents.modify.apply' : 'agents.modify.connect'
    : connectionState === 'needs-update' ? 'agents.modify.repair' : 'agents.modify.update');
  const configurationWriteBlocked = loading || !activeStatus || connectionState === 'invalid';
  const formValues: AgentFormValues = {
    model: isClaudeModelMappingClient ? '' : selectedModel,
    oauthConfiguration: selected === 'codex' && oauthConfiguration,
    mappings: isClaudeModelMappingClient ? claudeModelMappingsDraft : null,
  };
  const hasPendingChanges = Boolean(formEditBaselineByClient[selected]);

  const trackFormEdit = (nextValues: AgentFormValues) => {
    setConfigurationNotice('');
    setClearNotice('');
    setFormEditBaselineByClient((current) => {
      const baseline = current[selected] ?? formValues;
      const next = { ...current };
      if (sameAgentFormValues(baseline, nextValues)) delete next[selected];
      else next[selected] = baseline;
      agentFormEditBaselineCache = next;
      return next;
    });
  };

  const clearPendingChanges = () => {
    setFormEditBaselineByClient((current) => {
      const next = { ...current };
      delete next[selected];
      agentFormEditBaselineCache = next;
      return next;
    });
  };
  const claudeMappingsReady = !isClaudeModelMappingClient
    || (selected === 'claude-desktop' ? !desktopValidation : claudeMappingRoles.every((role) =>
      Boolean(findAgentModel(models, claudeModelMappingsDraft[role.key])),
    ));
  const claudeCodeRuntimeSettingsReady = selected !== 'claude-code' || (
    claudeModelMappingsDraft.maxContextTokens >= 100_000
    && claudeModelMappingsDraft.maxContextTokens <= 1_000_000
    && claudeModelMappingsDraft.autoCompactPct >= 1
    && claudeModelMappingsDraft.autoCompactPct <= 100
  );
  const canConfigureActiveClient = Boolean(
    activeStatus?.supportedPlatform
      && (activeStatus.installed || activeStatus.configExists),
  );
  const canEnable = Boolean(
    canConfigureActiveClient
      && !modelLoading
      && (isClaudeModelMappingClient
        ? claudeMappingsReady && claudeCodeRuntimeSettingsReady
        : selectedModelOption),
  );
  const activeLaunchTargets = activeStatus?.launchTargets ?? [];
  const launchEnabled = Boolean(
    activeStatus?.supportedPlatform
      && activeStatus.installed,
  );
  const activeLaunchDirectoryHistory = launchDirectoryHistory[selected] ?? [];
  const deepSeekHarnessLaunchModeLabel = {
    web: t('agents.deepseekLaunch.mode.web'),
    headless: t('agents.deepseekLaunch.mode.headless'),
    acp: t('agents.deepseekLaunch.mode.acp'),
    sdk: t('agents.deepseekLaunch.mode.sdk'),
    'sdk-minimal': t('agents.deepseekLaunch.mode.sdkMinimal'),
    custom: t('agents.deepseekLaunch.mode.custom'),
  }[deepSeekHarnessLaunchDraft.mode];
  const deepSeekHarnessLaunchModeDescription = {
    web: t('agents.deepseekLaunch.mode.webDescription'),
    headless: t('agents.deepseekLaunch.mode.headlessDescription'),
    acp: t('agents.deepseekLaunch.mode.acpDescription'),
    sdk: t('agents.deepseekLaunch.mode.sdkDescription'),
    'sdk-minimal': t('agents.deepseekLaunch.mode.sdkMinimalDescription'),
    custom: t('agents.deepseekLaunch.mode.customDescription'),
  }[deepSeekHarnessLaunchDraft.mode];
  const modelHint = (modelLoading
      ? t('agents.model.readingAvailable')
      : models.length === 0
        ? ''
        : activeStatus?.modificationState === 'applied'
          ? t('agents.model.current', { model: appliedModel || '—' })
          : t('agents.model.firstSelection', { count: models.length }));
  const modificationDescription = activeStatus?.modificationState === 'invalid'
    ? t('agents.modify.invalid')
    : nativeOauth ? t('agents.nativeOAuth.activeHint')
    : selected === 'copilot-cli' ? t('agents.modify.copilotCliHint')
    : selected === 'vscode-copilot' ? t('agents.modify.vscodeCopilotHint')
    : selected === 'antigravity-cli' ? t('agents.modify.antigravityCliHint')
    : selected === 'workbuddy'
      ? t('agents.modify.workbuddyHint')
    : selected === 'zcode'
      ? t('agents.modify.zcodeRestart')
      : '';
  const refreshModels = () => {
    void loadModels(selected);
  };

  const applySelectedConfiguration = () => {
    if (isPiClient) {
      void (activeStatus?.pluginInstalled ? repairPiProvider() : installPiProvider());
      return;
    }
    void applyConfigurationChanges();
  };

  const reloadStatusesAfterAction = async () => {
    setDetectionError('');
    try {
      await loadStatuses(true);
    } catch (requestError) {
      setDetectionError(String(requestError));
    }
  };

  const selectModel = (value: string) => {
    setConfigurationNotice('');
    setClearNotice('');
    const model = findAgentModel(models, value);
    if (!model) return;
    if (!isClaudeModelMappingClient) trackFormEdit({ ...formValues, model: model.name });
    setModelSelectionError('');
    setModelByClient((current) => {
      const next = { ...current, [selected]: model.name };
      writeAgentModelSelections(next);
      return next;
    });
  };

  const editClaudeModelMappings = (update: (current: ClaudeModelMappings) => ClaudeModelMappings) => {
    if (!isClaudeModelMappingClient) return;
    const next = update(claudeModelMappingsDraft);
    trackFormEdit({ ...formValues, mappings: next });
    if (!sameAgentModelMappings(claudeModelMappingsDraft, next)) {
      claudeModelMappingsDirtyRef.current[selected] = true;
    }
    setClaudeModelMappingsDraftByClient((current) => ({ ...current, [selected]: next }));
  };

  const selectEmbeddedModel = (value: string) => {
    const model = findAgentModel(models, value);
    if (!model) return;
    if (isClaudeModelMappingClient) {
      editClaudeModelMappings((current) => ({ ...current, sonnet: model.name }));
    }
    selectModel(model.name);
  };

  const selectClaudeModelMapping = (
    role: 'opus' | 'sonnet' | 'haiku' | 'fable',
    value: string,
  ) => {
    const model = findAgentModel(models, value);
    if (!model || !isClaudeModelMappingClient) return;
    setModelSelectionError('');
    editClaudeModelMappings((current) => followClaudeCodeContextWindow(
      { ...current, [role]: model.name }, current.maxContextTokens, selected === 'claude-code',
    ));
  };

  const changeClaude1mPreference = (
    preference: 'opus1m' | 'sonnet1m' | 'haiku1m' | 'fable1m',
    enabled: boolean,
  ) => {
    if (!isClaudeModelMappingClient) return;
    editClaudeModelMappings((current) => followClaudeCodeContextWindow(
      { ...current, [preference]: enabled }, current.maxContextTokens, selected === 'claude-code',
    ));
  };

  const changeClaudeCodeRuntimeSetting = (
    key: 'maxContextTokens' | 'autoCompactPct',
    value: number,
  ) => {
    if (selected !== 'claude-code') return;
    editClaudeModelMappings((current) => ({ ...current, [key]: value }));
  };

  const changeClaudeCodeAutoCompactDisabled = (disabled: boolean) => {
    if (selected !== 'claude-code') return;
    editClaudeModelMappings((current) => ({ ...current, disableAutoCompact: disabled }));
  };

  const changeClaudeCustomMapping = (enabled: boolean) => {
    if (!isClaudeModelMappingClient) return;
    setClaudeCustomMappingByClient((current) => ({ ...current, [selected]: enabled }));
    setModelSelectionError('');
    if (selected !== 'claude-code') return;
    editClaudeModelMappings((current) => ({
      ...current,
      opus: resolveAgentModelForAliasMode(models, current.opus, enabled),
      sonnet: resolveAgentModelForAliasMode(models, current.sonnet, enabled),
      haiku: resolveAgentModelForAliasMode(models, current.haiku, enabled),
    }));
  };

  const requireSelectedModel = () => {
    if (modelLoading) {
      setModelSelectionError(t('agents.error.modelsLoading'));
      return null;
    }
    if (models.length === 0) {
      setModelSelectionError(modelError || t('agents.error.noModels'));
      return null;
    }
    const model = findAgentModel(models, selectedModel);
    if (!model) {
      setModelSelectionError(t('agents.error.selectionGone'));
      return null;
    }
    setModelSelectionError('');
    return model.name;
  };

  const requireClaudeModelMappings = (): ClaudeModelMappings | null => {
    if (!isClaudeModelMappingClient) return null;
    if (selected === 'claude-desktop') {
      if (desktopValidation) {
        setModelSelectionError(t(`agents.claudeDesktopMapping.error.${desktopValidation}`));
        return null;
      }
      const selectedEntries = selectedDesktopModelEntries(resolvedDesktopEntries);
      return { ...createClaudeModelMappings(), sonnet: selectedEntries[0].model.trim(),
        desktopModels: selectedEntries.map((entry) => ({ ...entry, model: entry.model.trim(), alias: entry.alias.trim() })) };
    }
    const resolved = {} as ClaudeModelMappings;
    for (const role of claudeMappingRoles) {
      const model = findAgentModel(models, claudeModelMappingsDraft[role.key]);
      if (!model) {
        setModelSelectionError(t('agents.error.mappingSelectionGone'));
        return null;
      }
      resolved[role.key] = model.name;
      resolved[role.contextKey] = claudeModelMappingsDraft[role.contextKey];
    }
    if (selected === 'claude-code') {
      if (!claudeCodeRuntimeSettingsReady) {
        setModelSelectionError(t('agents.error.claudeCodeRuntimeSettingsInvalid'));
        return null;
      }
    }
    resolved.maxContextTokens = claudeModelMappingsDraft.maxContextTokens;
    resolved.autoCompactPct = claudeModelMappingsDraft.autoCompactPct;
    resolved.disableAutoCompact = claudeModelMappingsDraft.disableAutoCompact;
    resolved.startupModel = claudeModelMappingsDraft.startupModel.trim() || 'opus';
    resolved.subagentModel = claudeModelMappingsDraft.subagentModel.trim();
    resolved.manageDefaultModel = resolved.startupModel.length > 0;
    resolved.manageSubagentModel = resolved.subagentModel.length > 0;
    return resolved;
  };

  const handleOAuthLoginError = (requestError: unknown, action: OAuthLoginRequiredAction) => {
    const message = String(requestError);
    if (message.includes(CODEX_OAUTH_LOGIN_REQUIRED_ERROR)) {
      setOauthLoginRequiredAction(action);
      return true;
    }
    return false;
  };

  const clearCodexIntegration = async () => {
    setBusyAction('native-oauth');
    setConfigurationError('');
    setConfigurationNotice('');
    setClearNotice('');
    try {
      await invoke('restore_codex_official_config');
      setModelError('');
      setModelSelectionError('');
      setStatuses((current) => current.map((status) => status.id === 'codex'
        ? { ...status, codexNativeOauth: true } : status));
      await reloadStatusesAfterAction();
      setConfigurationNotice(t('agents.nativeOAuth.restored'));
    } catch (cause) {
      setConfigurationError(String(cause));
    } finally {
      setBusyAction(null);
    }
  };

  const closeConfigurationChanges = async () => {
    setBusyAction('close-config');
    setConfigurationError('');
    setConfigurationNotice('');
    setClearNotice('');
    try {
      await removeSelectedConfiguration();
      clearPendingChanges();
      if (isClaudeModelMappingClient) claudeModelMappingsDirtyRef.current[selected] = false;
      await reloadStatusesAfterAction();
      setModelSelectionError('');
      setModelError('');
      setConfigurationNotice(selected === 'codex' ? t('agents.modify.closed')
        : t('agents.clearIntegration.success', { name: activeDefinition.name }));
    } catch (cause) {
      setConfigurationError(String(cause));
    } finally {
      setBusyAction(null);
    }
  };

  const changeOauthConfiguration = async (enabled: boolean) => {
    setConfigurationNotice('');
    setClearNotice('');
    if (!enabled) {
      trackFormEdit({ ...formValues, oauthConfiguration: false });
      setOauthConfigurationDraft(false);
      return;
    }

    setBusyAction('oauth-check');
    try {
      await invoke('check_codex_oauth_login');
      trackFormEdit({ ...formValues, oauthConfiguration: true });
      setOauthConfigurationDraft(true);
    } catch (requestError) {
      if (!handleOAuthLoginError(requestError, 'enable')) {
        setConfigurationError(String(requestError));
      }
    } finally {
      setBusyAction(null);
    }
  };

  const applyConfigurationChanges = async () => {
    setConfigurationError('');
    const claudeModelMappings = requireClaudeModelMappings();
    if (isClaudeModelMappingClient && !claudeModelMappings) return;
    const model = isClaudeModelMappingClient
      ? claudeModelMappings?.sonnet ?? null
      : requireSelectedModel();
    if (!model) return;
    setBusyAction('apply');
    setConfigurationNotice('');
    try {
      const result = await invoke<AgentConfigActionResult>('update_agent_config', {
        client: selected,
        model,
        oauthConfiguration,
        claudeCodeModelMappings: selected === 'claude-code' ? claudeModelMappings : null,
        claudeDesktopModelMappings: selected === 'claude-desktop' ? claudeModelMappings : null,
        executableOverrides: executablePaths,
      });
      clearPendingChanges();
      if (isClaudeModelMappingClient) {
        claudeModelMappingsDirtyRef.current[selected] = false;
      }
      await reloadStatusesAfterAction();
      if (isDeepSeekHarnessClient) await loadModels('deepseek-harness');
      setOauthConfigurationDraft(null);
      setConfigurationNotice(t(result.outcome === 'unchanged' ? 'agents.backup.unchanged' : 'agents.backup.updated'));
      onConfigurationApplied?.();
    } catch (requestError) {
      if (!handleOAuthLoginError(requestError, 'apply')) {
        setConfigurationError(String(requestError));
      }
    } finally {
      setBusyAction(null);
    }
  };

  const installPiProvider = async () => {
    const model = requireSelectedModel();
    if (!model) return;
    setBusyAction('install-pi');
    setConfigurationError('');
    setConfigurationNotice('');
    setClearNotice('');
    try {
      await invoke<AgentConfigActionResult>('install_pi_provider', { model, executableOverrides: executablePaths });
      clearPendingChanges();
      await reloadStatusesAfterAction();
      setConfigurationNotice(t('agents.management.pluginInstalled'));
      onConfigurationApplied?.();
    } catch (requestError) {
      setConfigurationError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const updatePiProvider = async () => {
    const model = requireSelectedModel();
    if (!model) return;
    setBusyAction('update-pi');
    setConfigurationError('');
    setConfigurationNotice('');
    setClearNotice('');
    setPiProviderUpdateStatus(null);
    try {
      await invoke<AgentConfigActionResult>('update_pi_provider', { model, executableOverrides: executablePaths });
      clearPendingChanges();
      await reloadStatusesAfterAction();
      setConfigurationNotice(t('agents.management.pluginUpdated'));
      await loadPiProviderUpdateStatus();
    } catch (requestError) {
      setConfigurationError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const repairPiProvider = async () => {
    const model = requireSelectedModel();
    if (!model) return;
    setBusyAction('repair-pi');
    setConfigurationError('');
    try {
      const result = await invoke<AgentConfigActionResult>('repair_pi_provider', { model });
      clearPendingChanges();
      await reloadStatusesAfterAction();
      setConfigurationNotice(t(result.outcome === 'unchanged' ? 'agents.backup.unchanged' : 'agents.backup.updated'));
      onConfigurationApplied?.();
    } catch (requestError) {
      setConfigurationError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const uninstallPiProvider = async () => {
    setBusyAction('uninstall-pi');
    setConfigurationError('');
    setConfigurationNotice('');
    setClearNotice('');
    setPiProviderUpdateStatus(null);
    try {
      await invoke<AgentConfigActionResult>('uninstall_pi_provider', { executableOverrides: executablePaths });
      clearPendingChanges();
      await reloadStatusesAfterAction();
      setConfigurationNotice(t('agents.management.pluginUninstalled'));
    } catch (requestError) {
      setConfigurationError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const resetConfigurationToDefault = async () => {
    if (!templatePreview) return;
    const claudeModelMappings = requireClaudeModelMappings();
    if (isClaudeModelMappingClient && !claudeModelMappings) return;
    const model = isClaudeModelMappingClient
      ? claudeModelMappings?.sonnet ?? null
      : requireSelectedModel();
    if (!model) return;
    setBusyAction('default');
    setDefaultError('');
    try {
      await invoke<AgentConfigActionResult>('apply_agent_config_template', {
        revision: templatePreview.revision,
        client: selected,
        model,
        oauthConfiguration,
        claudeCodeModelMappings: selected === 'claude-code' ? claudeModelMappings : null,
        claudeDesktopModelMappings: selected === 'claude-desktop' ? claudeModelMappings : null,
      });
      clearPendingChanges();
      setDefaultConfirmOpen(false);
      if (isClaudeModelMappingClient) {
        claudeModelMappingsDirtyRef.current[selected] = false;
      }
      const refreshed = await invoke<AgentConfigStatus[]>('refresh_agent_config_statuses', { executableOverrides: executablePaths });
      setStatuses(refreshed);
      const current = refreshed.find((status) => status.id === selected)?.currentModel;
      setModelByClient((values) => { const next = { ...values, [selected]: current ?? '' }; writeAgentModelSelections(next); return next; });
      setConfigurationNotice(t('agents.backup.updated'));
      onConfigurationApplied?.();
      setOauthConfigurationDraft(null);
    } catch (requestError) {
      if (!handleOAuthLoginError(requestError, 'apply')) {
        const error = String(requestError);
        const setupError = selected === 'vscode-copilot' ? copilotSetupErrorKey(error) : undefined;
        setDefaultError(setupError ? t(setupError) : error);
        setTemplatePreview(null);
      }
    } finally {
      setBusyAction(null);
    }
  };

  const removeSelectedConfiguration = () => selected === 'codex'
    ? invoke<AgentConfigActionResult>('close_codex_config_modification')
    : invoke<AgentConfigActionResult>('set_agent_config_enabled', {
      client: selected, model: '', enabled: false, forceRestore: false,
      claudeCodeModelMappings: null, claudeDesktopModelMappings: null,
      executableOverrides: executablePaths,
    });

  const clearConfiguration = async () => {
    setBusyAction('clear');
    setConfigurationError('');
    setConfigurationNotice('');
    setClearError('');
    setClearNotice('');
    try {
      if (selected === 'codex') await invoke<string[]>('clear_codex_config');
      else await removeSelectedConfiguration();
      clearPendingChanges();
      if (isClaudeModelMappingClient) claudeModelMappingsDirtyRef.current[selected] = false;
      setClearConfirmOpen(false);
      setModelSelectionError('');
      setModelError('');
      setClearNotice(selected === 'codex' ? t('agents.clear.success')
        : t('agents.clearIntegration.success', { name: activeDefinition.name }));
      await reloadStatusesAfterAction();
      setOauthConfigurationDraft(null);
    } catch (requestError) {
      setClearError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const openLaunchDirectoryDialog = (target: AgentLaunchTarget) => {
    setLaunchDirectory(activeLaunchDirectoryHistory[0] ?? '');
    setLaunchDirectoryTarget(target);
    setLaunchDirectoryError('');
    setLaunchDirectoryDialogOpen(true);
  };

  const updateDeepSeekHarnessLaunchDraft = (update: Partial<DeepSeekHarnessLaunchDraft>) => {
    setDeepSeekHarnessLaunchDraft((current) => ({ ...current, ...update }));
    setLaunchDirectoryError('');
  };

  const chooseLaunchDirectory = async () => {
    setBusyAction('directory');
    setLaunchDirectoryError('');
    try {
      const selectedDirectory = await open({
        directory: true,
        multiple: false,
        defaultPath: launchDirectory || undefined,
        title: t('agents.launchDirectory.dialogTitle', { client: activeDefinition.name }),
      });
      if (typeof selectedDirectory === 'string') setLaunchDirectory(selectedDirectory);
    } catch (requestError) {
      setLaunchDirectoryError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const chooseExecutablePath = async (desktop = false) => {
    try {
      const selectedPath = await open({ directory: false, multiple: false, title: t('agents.executablePath.dialogTitle') });
      if (typeof selectedPath !== 'string' || !selectedPath.trim()) return;
      const key = desktop ? `${selected}:app` as const : selected;
      const next = { ...executablePaths, [key]: selectedPath.trim() };
      setExecutablePaths(next);
      writeAgentExecutablePaths(next);
      const refreshed = await invoke<AgentConfigStatus[]>('refresh_agent_config_statuses', { executableOverrides: next });
      setStatuses(refreshed);
      setConfigurationNotice(t('agents.executablePath.saved'));
    } catch (requestError) {
      setConfigurationError(String(requestError));
    }
  };

  const clearExecutablePath = (desktop = false) => {
    const next = { ...executablePaths };
    delete next[desktop ? `${selected}:app` as const : selected];
    setExecutablePaths(next);
    writeAgentExecutablePaths(next);
    void invoke<AgentConfigStatus[]>('refresh_agent_config_statuses', { executableOverrides: next }).then(setStatuses);
  };

  const rememberLaunchDirectory = (client: AgentClientId, directory: string) => {
    setLaunchDirectoryHistory((current) => {
      const next = rememberAgentLaunchDirectory(current, client, directory);
      writeAgentLaunchDirectoryHistory(next);
      return next;
    });
  };

  const invokeAgentLaunch = async (
    target: AgentLaunchTarget,
    workingDirectory: string | null = null,
    deepSeekHarnessOptions: DeepSeekHarnessLaunchOptions | null = null,
  ) => {
    const action = hasIndependentCliAndApp
      ? target.id === 'cli' ? 'launch-cli' : 'launch-app'
      : 'launch';
    setBusyAction(action);
    setLaunchError('');
    try {
      await invoke('launch_agent', {
        client: selected,
        target: target.id,
        executablePath: executablePaths[selected] ?? null,
        desktopExecutablePath: executablePaths[`${selected}:app`] ?? null,
        workingDirectory,
        deepseekHarnessOptions: deepSeekHarnessOptions,
      });
      if (selected === 'deepseek-harness') {
        await loadDeepSeekHarnessProcessStatus();
      }
      if (workingDirectory) {
        rememberLaunchDirectory(selected, workingDirectory);
        setLaunchDirectoryDialogOpen(false);
        setLaunchDirectoryTarget(null);
      }
    } catch (requestError) {
      if (workingDirectory) {
        setLaunchDirectoryError(String(requestError));
      } else if (!handleOAuthLoginError(requestError, 'launch')) {
        setLaunchError(String(requestError));
      }
    } finally {
      setBusyAction(null);
    }
  };

  const launchAgent = async (target: AgentLaunchTarget | null) => {
    if (!target) return;
    if (target.id === 'cli' || selected === 'vscode-copilot') {
      openLaunchDirectoryDialog(target);
      return;
    }
    await invokeAgentLaunch(target);
  };

  const stopDeepSeekHarness = async () => {
    setBusyAction('stop-deepseek');
    setLaunchError('');
    try {
      const status = await invoke<DeepSeekHarnessProcessStatus>('stop_deepseek_harness_process');
      setDeepSeekHarnessProcessStatus(status);
    } catch (requestError) {
      setLaunchError(String(requestError));
    } finally {
      setBusyAction(null);
    }
  };

  const restartDesktopApp = async () => {
    if (!['codex', 'opencode', 'claude-desktop', 'zcode', 'workbuddy'].includes(selected)) return;
    setBusyAction('restart-app');
    setLaunchError('');
    try {
      await invoke('restart_agent_app', { client: selected,
        executablePath: executablePaths[hasIndependentCliAndApp ? `${selected}:app` as const : selected] ?? null });
    } catch (requestError) {
      if (!handleOAuthLoginError(requestError, 'launch')) {
        setLaunchError(String(requestError));
      }
    } finally {
      setBusyAction(null);
    }
  };

  useEffect(() => {
    const pending = codexCliServiceRestart;
    if (!pending) {
      setCliServiceRestarting(false);
      return;
    }
    let active = true;
    pending.then(
      () => { if (active) updateViewStateForClient('codex', { configurationNotice: t('agents.launch.cliServiceRestarted') }); },
      (requestError) => { if (active) updateViewStateForClient('codex', { launchError: String(requestError) }); },
    ).finally(() => { if (active) setCliServiceRestarting(false); });
    return () => { active = false; };
  }, []);

  const restartCodexCliService = async () => {
    if (selected !== 'codex' || cliServiceRestarting || codexCliServiceRestart) return;
    const executablePath = executablePaths[selected] ?? null;
    setCliServiceRestarting(true);
    setLaunchError('');
    setConfigurationNotice('');
    try {
      if (!await invoke<boolean>('get_codex_cli_daemon_running', { executablePath })) {
        setConfigurationNotice(t('agents.launch.cliServiceNotRunning'));
        return;
      }
      const confirmed = await askConfirmation({
        title: t('agents.launch.restartCliTitle'),
        message: `${t('agents.launch.restartCliMessage')} ${t('agents.launch.restartCliWarning')}`,
        confirmText: t('agents.launch.restartCliConfirm'),
        variant: 'danger',
      });
      if (!confirmed) return;
      const restart = invoke<void>('restart_codex_cli_daemon', { executablePath })
        .finally(() => { codexCliServiceRestart = null; });
      codexCliServiceRestart = restart;
      await restart;
      setConfigurationNotice(t('agents.launch.cliServiceRestarted'));
    } catch (requestError) {
      setLaunchError(String(requestError));
    } finally {
      setCliServiceRestarting(false);
    }
  };

  const restartDeepSeekHarness = async () => {
    setBusyAction('restart-deepseek');
    setLaunchError('');
    try {
      setDeepSeekHarnessProcessStatus(await invoke<DeepSeekHarnessProcessStatus>('restart_deepseek_harness_process'));
    } catch (requestError) {
      setLaunchError(String(requestError));
      await loadDeepSeekHarnessProcessStatus().catch(() => undefined);
    } finally {
      setBusyAction(null);
    }
  };

  const launchFromDirectoryDialog = async () => {
    if (!launchDirectoryTarget) return;
    const workingDirectory = launchDirectory.trim();
    if (!workingDirectory) {
      setLaunchDirectoryError(t('agents.launchDirectory.directoryRequired'));
      return;
    }
    let deepSeekHarnessOptions: DeepSeekHarnessLaunchOptions | null = null;
    if (isDeepSeekHarnessClient) {
      const result = buildDeepSeekHarnessLaunchOptions(deepSeekHarnessLaunchDraft);
      if (result.error) {
        const errorKeys = {
          invalidPort: 'agents.deepseekLaunch.error.invalidPort',
          taskRequired: 'agents.deepseekLaunch.error.taskRequired',
          profileRequired: 'agents.deepseekLaunch.error.profileRequired',
          invalidProfile: 'agents.deepseekLaunch.error.invalidProfile',
        } as const;
        setLaunchDirectoryError(t(errorKeys[result.error]));
        return;
      }
      deepSeekHarnessOptions = result.options;
    }
    setLaunchDirectoryError('');
    await invokeAgentLaunch(launchDirectoryTarget, workingDirectory, deepSeekHarnessOptions);
  };

  const createManualBackup = async () => {
    setBusyAction('backup'); setConfigurationError(''); setConfigurationNotice('');
    try {
      await invoke('create_agent_config_backup', { client: selected });
      setConfigurationNotice(t('agents.backup.created'));
    } catch (cause) { setConfigurationError(String(cause)); }
    finally { setBusyAction(null); }
  };

  const openDefaultConfirmation = async () => {
    const mappings = requireClaudeModelMappings();
    if (isClaudeModelMappingClient && !mappings) return;
    const model = isClaudeModelMappingClient ? mappings?.sonnet : requireSelectedModel();
    if (!model) return;
    setDefaultError(''); setConfigurationError(''); setTemplatePreview(null); setBusyAction('default');
    try {
      const preview = await invoke<{ revision: string; files: string[] }>('preview_agent_config_template', {
        client: selected, model, oauthConfiguration,
        claudeCodeModelMappings: selected === 'claude-code' ? mappings : null,
        claudeDesktopModelMappings: selected === 'claude-desktop' ? mappings : null,
      });
      setTemplatePreview(preview); setDefaultConfirmOpen(true);
    } catch (cause) { setConfigurationError(String(cause)); }
    finally { setBusyAction(null); }
  };

  const closeDefaultConfirmation = () => {
    setDefaultError('');
    setDefaultConfirmOpen(false);
  };

  const openClearConfirmation = () => {
    setClearError('');
    setClearConfirmOpen(true);
  };

  const closeClearConfirmation = () => {
    setClearError('');
    setClearConfirmOpen(false);
  };

  const openClearIntegrationGuide = () => {
    setOauthLoginRequiredAction(null);
    setActiveSubpage('management');
    requestAnimationFrame(() => document.getElementById('agent-clear-integration')?.focus());
  };

  const availableSubpages = agentSubpages.filter(
    (subpage) => (!subpage.clients || subpage.clients.includes(selected)) && (!embedded || subpage.id !== 'sessions'),
  );
  const oauthLoginRequiredDescription = oauthLoginRequiredAction === 'enable' ? (
    <>
      {t('agents.oauthLoginRequired.enableDescription')}
      <strong>{t('agents.tabs.management')}</strong>
      {t('agents.oauthLoginRequired.enableManagementConnector')}
      <strong>{t('agents.oauthLoginRequired.enableClearConfiguration')}</strong>
      {t('agents.oauthLoginRequired.enableDescriptionSuffix')}
    </>
  ) : oauthLoginRequiredAction ? t(`agents.oauthLoginRequired.${oauthLoginRequiredAction}Description`) : '';

  const codexCatalogButton = selected === 'codex' ? (
    <button type="button" className="secondary-button agent-codex-catalog-button"
      onClick={() => setCodexCatalogDialogOpen(true)} disabled={busy}>
      <SlidersHorizontal size={16} />
      {t('agents.catalog.button')}
    </button>
  ) : null;

  const harnessCatalogButton = isDeepSeekHarnessClient ? (
    <button type="button" className="secondary-button agent-codex-catalog-button"
      onClick={() => setHarnessCatalogDialogOpen(true)} disabled={busy || configurationWriteBlocked}>
      <SlidersHorizontal size={16} />{t('agents.harness.button')}
    </button>
  ) : null;

  const closeConfigurationButton = canCloseConfiguration ? (
    <button type="button" className="secondary-button agent-close-configuration"
      title={t(selected === 'codex' ? 'agents.modify.closeHint' : 'agents.clearIntegration.description')}
      disabled={busy || configurationWriteBlocked}
      onClick={() => void closeConfigurationChanges()}>
      {busyAction === 'close-config' ? <LoaderCircle size={16} className="spin" /> : null}
      {t('agents.modify.close')}
    </button>
  ) : null;

  const configurationFeedback = <AgentConfigurationFeedback pending={hasPendingChanges}
    status={modelLoading || !activeStatus ? t('agents.modify.checking')
      : activeStatus.modificationState === 'invalid' ? t('agents.modify.invalidState') : ''}
    description={modificationDescription} />;
  const setupErrorKey = selected === 'vscode-copilot' ? copilotSetupErrorKey(configurationError) : undefined;
  const configurationErrorMessage = (setupErrorKey ? t(setupErrorKey) : configurationError) || (!nativeOauth ? modelSelectionError || (selected === 'claude-code' ? '' : modelError) : '');

  const editDesktopEntries = (entries: ClaudeDesktopModelMapping[]) => {
    setModelSelectionError('');
    editClaudeModelMappings((current) => ({ ...current, desktopModels: entries }));
  };
  const updateDesktopEntry = (index: number, changes: Partial<ClaudeDesktopModelMapping>) => {
    editDesktopEntries(desktopEntries.map((entry, i) => i === index ? { ...entry, ...changes } : entry));
  };
  const desktopModelEditor = (
    <section className="agent-core-setting-section agent-desktop-models">
      <div className="agent-section-heading">
        <strong>{t('agents.claudeDesktopMapping.title')}</strong>
        <div className="agent-section-heading-actions">
          <label
            className="agent-claude-desktop-mapping-filter"
            title={t('agents.claudeDesktopMapping.customMappingHint')}
          >
            <span>{t('agents.claudeDesktopMapping.customMapping')}</span>
            <span className="switch-control">
              <input
                type="checkbox"
                checked={claudeCustomMapping}
                onChange={(event) => changeClaudeCustomMapping(event.currentTarget.checked)}
                disabled={busy || loading || modelLoading}
              />
              <span className="switch-track" />
            </span>
          </label>
          <button type="button" className="secondary-button compact-button" onClick={() => setDesktopHelpOpen(true)}>
            {t('agents.claudeDesktopMapping.help')}
          </button>
          <button type="button" className="primary-button compact-button" disabled={busy || loading}
            onClick={() => editDesktopEntries([...desktopEntries, { model: '', alias: '', context1m: false }])}>
            {t('agents.claudeDesktopMapping.add')}
          </button>
        </div>
      </div>
      {!desktopEntries.length ? <p className="agent-model-hint">{t('agents.claudeDesktopMapping.empty')}</p> : null}
      <div className="agent-desktop-model-list">
        {resolvedDesktopEntries.map((entry, index) => {
          const entryError = entry.model.trim() ? desktopEntryValidation(entry) : null;
          const aliasNotice = desktopAliasNotice(entry, resolvedDesktopEntries, models, appliedDesktopEntries);
          const aliasSuffixed = desktopEntries[index].alias.trim().toLowerCase() !== entry.alias.trim().toLowerCase();
          const isClaude = isClaudeDesktopModel(entry.model);
          return (
          <div className="agent-claude-desktop-mapping-row agent-desktop-model-row" key={index}>
            <div className="agent-desktop-model-fields">
              <div className="agent-desktop-model-field">
                <span>{t('agents.claudeDesktopMapping.model')}</span>
                <AgentModelPicker models={claudeMappingModels} value={entry.model} loading={modelLoading} error={modelError}
                  allowCustomValue disabled={busy || loading || !activeStatus?.installed || !activeStatus.supportedPlatform}
                  onChange={(model) => updateDesktopEntry(index, { model, ...(isClaudeDesktopModel(model) ? { alias: '' } : {}) })}
                  onRefresh={refreshModels} />
              </div>
              <div className="agent-desktop-model-field">
                <span>{t('agents.claudeDesktopMapping.alias')}</span>
                <AgentModelPicker
                  models={claudeDesktopAliasSuggestions.filter((alias) => !resolvedDesktopEntries.some((other, i) => i !== index && other.model.trim() && desktopModelId(other).toLowerCase() === alias))
                    .map((name) => ({ name, alias: t('agents.claudeDesktopMapping.suggestedAlias') }))}
                  value={entry.alias} loading={false} error="" disabled={busy || loading}
                  editable={{ label: t('agents.claudeDesktopMapping.alias'),
                    placeholder: t(isClaude ? 'agents.claudeDesktopMapping.claudeDefault' : 'agents.claudeDesktopMapping.noRename'), maxLength: 128 }}
                  onChange={(alias) => updateDesktopEntry(index, { alias })} />
              </div>
              <div className="agent-section-heading-actions agent-desktop-model-actions">
                <label className="agent-claude-context-toggle" title={t('agents.claudeMapping.context1mHint')}>
                  <span>{t('agents.claudeMapping.context1m')}</span>
                  <span className="switch-control"><input type="checkbox" checked={entry.context1m}
                    onChange={(event) => updateDesktopEntry(index, { context1m: event.currentTarget.checked })}
                    disabled={busy || loading} /><span className="switch-track" /></span>
                </label>
                <button type="button" className="icon-button quiet" aria-label={t('agents.claudeDesktopMapping.remove')}
                  title={t('agents.claudeDesktopMapping.remove')} disabled={busy || loading}
                  onClick={() => editDesktopEntries(desktopEntries.filter((_, i) => i !== index))}>
                  <Trash2 size={16} />
                </button>
              </div>
            </div>
            {aliasSuffixed
              ? <p className="agent-desktop-model-notice" role="status">{t('agents.claudeDesktopMapping.aliasSuffixed', { alias: entry.alias.trim() })}</p>
              : entryError && hasPendingChanges && !modelLoading
              ? <p className="agent-inline-message warning agent-desktop-model-notice" role="status">{t(`agents.claudeDesktopMapping.error.${entryError}`)}</p>
              : aliasNotice === 'aliasExists'
                ? <p className="agent-inline-message warning agent-desktop-model-notice" role="status">{t(
                  claudeDesktopDefaultAliases.includes(entry.alias.trim().toLowerCase())
                    ? 'agents.claudeDesktopMapping.defaultAliasExists' : 'agents.claudeDesktopMapping.error.aliasExists',
                  { alias: entry.alias.trim() },
                )}</p>
                : aliasNotice === 'sameModel'
                  ? <p className="agent-desktop-model-notice" role="status">{t('agents.claudeDesktopMapping.sameModel')}</p>
                  : isClaude
                    ? <p className="agent-desktop-model-notice" role="status">{t('agents.claudeDesktopMapping.claudeDefault')}</p>
                    : null}
          </div>
          );
        })}
      </div>
      {desktopValidation === 'duplicate' && !modelLoading && hasPendingChanges
        ? <p className="agent-inline-message warning" role="status">{t(`agents.claudeDesktopMapping.error.${desktopValidation}`)}</p> : null}
    </section>
  );

  return (
    <section className={`page management-page agents-page${embedded ? ' agents-page-embedded' : ''}`}>
      {embedded ? (
        <header className="management-header">
          <div className="agent-embedded-header-copy">
            <h1>{t('agents.embedded.title')}</h1>
            <p>{t('agents.embedded.subtitle')}</p>
          </div>
        </header>
      ) : null}

      <div className="agent-workbench">
        <AgentClientList clients={agentDefinitions.map((agent) => {
          const status = statuses.find((item) => item.id === agent.id);
          return {
            id: agent.id,
            name: agent.name,
            icon: <AgentMark definition={agent} />,
            summary: listStatusText(status),
            installed: Boolean(status?.installed),
            detected: Boolean(status && (status.installed || status.configExists || status.configured
              || (agent.id === 'pi' && status.pluginInstalled))),
          };
        })} selected={selected} onSelect={(client) => {
          if (client !== selected) requestAppNavigation(() => setSelected(client));
        }} onRefresh={() => void refresh()}
          loading={loading} busy={busy} error={detectionError} onDismissError={() => setDetectionError('')} />

        <section className="panel agent-config-panel">
          {availableSubpages.length > 1 ? (
            <div className="agent-subpage-tabs" role="tablist" aria-label={t('agents.tabs.label')}>
            {availableSubpages.map((subpage) => (
              <button
                type="button"
                id={`agent-subpage-tab-${subpage.id}`}
                role="tab"
                className={activeSubpage === subpage.id ? 'active' : ''}
                aria-selected={activeSubpage === subpage.id}
                aria-controls={`agent-subpage-panel-${subpage.id}`}
                tabIndex={activeSubpage === subpage.id ? 0 : -1}
                key={subpage.id}
                disabled={busy}
                onClick={() => setActiveSubpage(subpage.id)}
                onKeyDown={(event) => {
                  const index = availableSubpages.findIndex((item) => item.id === subpage.id);
                  const next = event.key === 'Home' ? 0 : event.key === 'End' ? availableSubpages.length - 1
                    : event.key === 'ArrowRight' ? (index + 1) % availableSubpages.length
                    : event.key === 'ArrowLeft' ? (index - 1 + availableSubpages.length) % availableSubpages.length : -1;
                  if (next < 0) return;
                  event.preventDefault();
                  setActiveSubpage(availableSubpages[next].id);
                  document.getElementById('agent-subpage-tab-' + availableSubpages[next].id)?.focus();
                }}
              >
                {t(subpage.labelKey)}
              </button>
            ))}
          </div>
          ) : null}
          <div className="agent-config-scroll-region">
          {embedded && selected === 'vscode-copilot' && activeSubpage === 'core' && activeStatus ? (
            <CopilotPluginStatus installed={activeStatus.pluginInstalled} version={activeStatus.pluginVersion} />
          ) : null}
          {embedded && activeSubpage === 'core' ? (
            <div className="agent-minimal-config" id="agent-subpage-panel-core" role="tabpanel" aria-labelledby="agent-subpage-tab-core">
              <div className="agent-minimal-client-summary">
                <span className="agent-minimal-client-icon"><AgentMark definition={activeDefinition} size={24} /></span>
                <div>
                  <strong>{activeDefinition.name}</strong>
                  <span>{activeStatus?.installed ? t('agents.clientDetected') : t('agents.clientNotDetected')}</span>
                </div>
                <span className="agent-minimal-version" title={activeStatus?.version ?? undefined}>
                  {activeStatus?.version ?? activeStatus?.appVersion ?? activeStatus?.cliVersion ?? t('agents.notFetched')}
                </span>
              </div>

                <MessageNotice message={activeStatus?.error || activeStatus?.warnings.join('；')} tone={activeStatus?.error ? 'error' : 'info'} />
                {activeStatus && !activeStatus.installed ? <div className="agent-detection-help" role="status">
                  <span>{t('agents.executablePath.notDetected')}</span>
                  <button type="button" className="link-button" onClick={() => setActiveSubpage('management')}>{t('agents.executablePath.openManagement')}</button>
                </div> : null}
              {selected === 'claude-desktop' ? desktopModelEditor : <div className="agent-minimal-field">
                <label htmlFor="embedded-agent-model">{t(isDeepSeekHarnessClient ? 'agents.harness.defaultModel' : 'agents.useModel')}</label>
                <AgentModelPicker
                  models={isClaudeModelMappingClient ? claudeMappingModels : models}
                  value={isClaudeModelMappingClient ? claudeModelMappingsDraft.sonnet : savedSelectedModel}
                  preserveValue={!isClaudeModelMappingClient}
                  loading={modelLoading}
                  error={modelError}
                  disabled={busy || !canConfigureActiveClient}
                  onChange={selectEmbeddedModel}
                  onRefresh={refreshModels}
                />
                {codexCatalogButton}{harnessCatalogButton}
                {!isClaudeModelMappingClient && missingModelHint ? <span className="agent-model-hint" role="status">{missingModelHint}</span> : null}
              </div>}

              {isDeepSeekHarnessClient ? <p className="agent-model-hint">{t('agents.harness.defaultHint')}</p> : null}

              <div className="agent-save-bar">
                {configurationFeedback}
                <div className={`agent-save-actions${!isPiClient ? " agent-codex-save-actions" : ""}`}>
                  {closeConfigurationButton}
                  <button
                    type="button"
                    className="primary-button"
                    onClick={applySelectedConfiguration}
                    disabled={busy || !canEnable || configurationWriteBlocked}
                  >
                    {['apply', 'install-pi', 'repair-pi'].includes(busyAction ?? '') ? <LoaderCircle size={16} className="spin" /> : null}
                    {isPiClient
                      ? activeStatus?.pluginInstalled ? configurationActionLabel : t('agents.pi.install')
                      : configurationActionLabel}
                  </button>
                </div>
              </div>

            </div>
          ) : null}

          {!embedded && activeSubpage === 'core' ? (
            <div
              className="agent-core-config"
              id="agent-subpage-panel-core"
              role="tabpanel"
              aria-labelledby="agent-subpage-tab-core"
            >
              <div className={`agent-status-grid ${hasIndependentCliAndApp ? 'dual-install-status-grid' : isPiClient || selected === 'vscode-copilot' ? 'pi-status-grid' : ''}`}>
                <div>
                  <span><BadgeCheck size={14} />{t('agents.installStatus')}</span>
                  <strong>{activeStatus?.installed ? t('agents.clientDetected') : t('agents.clientNotDetected')}</strong>
                </div>
                {hasIndependentCliAndApp ? (
                  <>
                    <div>
                      <span>{t('agents.cliVersion')}</span>
                      <strong title={activeStatus?.cliVersion ?? undefined}>{activeStatus?.cliVersion ?? t('agents.notFetched')}</strong>
                    </div>
                    <div>
                      <span>{t('agents.appVersion')}</span>
                      <strong title={activeStatus?.appVersion ?? undefined}>{activeStatus?.appVersion ?? t('agents.notFetched')}</strong>
                    </div>
                  </>
                ) : isPiClient ? (
                  <>
                    <div>
                      <span>{t('agents.clientVersion')}</span>
                      <strong title={activeStatus?.version ?? undefined}>{activeStatus?.version ?? t('agents.notFetched')}</strong>
                    </div>
                    <div>
                      <span>
                        {t('agents.pluginVersion')}
                        {piPluginUpdateAvailable ? (
                          <span
                            className="agent-version-update-label"
                            role="status"
                            aria-label={piPluginUpdateTitle}
                            title={piPluginUpdateTitle}
                          >
                            {t('agents.pi.updateAvailableShort')}
                          </span>
                        ) : null}
                      </span>
                      <strong title={piPluginUpdateTitle}>{activeStatus?.pluginVersion ?? t('agents.notFetched')}</strong>
                    </div>
                  </>
                ) : (
                  <div>
                    <span>{t('agents.clientVersion')}</span>
                    <strong title={activeStatus?.version ?? undefined}>{activeStatus?.version ?? t('agents.notFetched')}</strong>
                  </div>
                )}
                {selected === 'vscode-copilot' ? <CopilotPluginStatus standalone={false} installed={Boolean(activeStatus?.pluginInstalled)} version={activeStatus?.pluginVersion ?? null} /> : null}
              </div>

              <MessageNotice message={activeStatus?.error || activeStatus?.warnings.join('；')} tone={activeStatus?.error ? 'error' : 'info'} />
              {activeStatus && !activeStatus.installed ? <div className="agent-detection-help" role="status">
                <span>{t('agents.executablePath.notDetected')}</span>
                <button type="button" className="link-button" onClick={() => setActiveSubpage('management')}>{t('agents.executablePath.openManagement')}</button>
              </div> : null}

              {!isClaudeModelMappingClient ? (
                <section className="agent-core-setting-section agent-model-section">
                  <div className="agent-section-heading">
                    <div><strong>{t(isDeepSeekHarnessClient ? 'agents.harness.defaultModel' : 'agents.useModel')}</strong></div>
                  </div>
                  <AgentModelPicker
                    models={models}
                    value={savedSelectedModel}
                    preserveValue
                    loading={modelLoading}
                    error={modelError}
                    disabled={busy || !canConfigureActiveClient}
                    onChange={selectModel}
                    onRefresh={refreshModels}
                  />
                  {modelHint ? <span className="agent-model-hint agent-model-status" title={modelHint} aria-live="polite">{modelHint}</span> : null}
                  {missingModelHint ? <span className="agent-model-hint" role="status">{missingModelHint}</span> : null}
                  {isDeepSeekHarnessClient ? <p className="agent-model-hint">{t('agents.harness.defaultHint')}</p> : null}
                  {harnessCatalogButton}
                  {selected === 'codex' ? (
                    <div className="agent-codex-options">
                      <div className="agent-auth-method">
                        <div className="agent-signin-label">
                          <span id="agent-connection-method-label" className="agent-signin-label-title">{t('agents.modify.authMethod')}</span>
                          <button type="button" className="agent-signin-help-toggle" aria-expanded={connectionHelpOpen} aria-controls="agent-signin-hint"
                            onClick={() => updateViewState({ connectionHelpOpen: !connectionHelpOpen })}>
                            {t(connectionHelpOpen ? 'agents.modify.authHelpHide' : 'agents.modify.authHelpShow')}
                            <ChevronDown size={14} aria-hidden="true" />
                          </button>
                        </div>
                        <div
                          id="agent-connection-method"
                          className="agent-auth-segmented"
                          role="radiogroup"
                          aria-labelledby="agent-connection-method-label"
                          aria-describedby={connectionHelpOpen ? "agent-signin-hint" : undefined}
                          onKeyDown={(event) => {
                            if (busy || loading || modelLoading) return;
                            if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
                              event.preventDefault();
                              if (!oauthConfiguration) void changeOauthConfiguration(true);
                            } else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
                              event.preventDefault();
                              if (oauthConfiguration) void changeOauthConfiguration(false);
                            }
                          }}
                        >
                          <button
                            type="button"
                            role="radio"
                            className={`agent-auth-segmented-button${!oauthConfiguration ? ' active' : ''}`}
                            aria-checked={!oauthConfiguration}
                            tabIndex={!oauthConfiguration ? 0 : -1}
                            disabled={busy || loading || modelLoading}
                            data-value="apikey"
                            onClick={() => {
                              if (oauthConfiguration) {
                                void changeOauthConfiguration(false);
                              }
                            }}
                          >
                            {t('agents.modify.authApiKey')}
                          </button>
                          <button
                            type="button"
                            role="radio"
                            className={`agent-auth-segmented-button${oauthConfiguration ? ' active' : ''}`}
                            aria-checked={Boolean(oauthConfiguration)}
                            tabIndex={oauthConfiguration ? 0 : -1}
                            disabled={busy || loading || modelLoading}
                            data-value="oauth"
                            onClick={() => {
                              if (!oauthConfiguration) {
                                void changeOauthConfiguration(true);
                              }
                            }}
                          >
                            {busyAction === 'oauth-check' ? <LoaderCircle size={14} className="spin" /> : null}
                            {t('agents.modify.authOAuth')}
                          </button>
                        </div>
                      </div>

                      {codexCatalogButton}
                      <div id="agent-signin-hint" className="agent-signin-explanation" hidden={!connectionHelpOpen}>
                        <dl>
                          <div><dt>{t('agents.modify.authApiKey')}</dt><dd>{t('agents.modify.authApiKeyHint')}</dd></div>
                          <div><dt>{t('agents.modify.authOAuth')}</dt><dd>{t('agents.modify.authOAuthHint')}</dd></div>
                        </dl>
                      </div>
                    </div>
                  ) : null}
                </section>
              ) : null}

              {selected === 'claude-desktop' ? desktopModelEditor : null}
              {selected === 'claude-code' ? (
                <section className="agent-core-setting-section agent-claude-desktop-mapping">
                  <div className="agent-section-heading">
                    <div>
                      <strong>{t(selected === 'claude-code'
                        ? 'agents.claudeCodeMapping.title'
                        : 'agents.claudeDesktopMapping.title')}</strong>

                    </div>
                    <div className="agent-section-heading-actions">
                      <label
                        className="agent-claude-desktop-mapping-filter"
                        title={t('agents.claudeDesktopMapping.customMappingHint')}
                      >
                        <span>{t('agents.claudeDesktopMapping.customMapping')}</span>
                        <span className="switch-control">
                          <input
                            type="checkbox"
                            checked={claudeCustomMapping}
                            onChange={(event) => changeClaudeCustomMapping(event.currentTarget.checked)}
                            disabled={busy || loading || modelLoading}
                          />
                          <span className="switch-track" />
                        </span>
                      </label>
                    </div>
                  </div>
                  <ClaudeCodeModelInputs
                        routes={claudeModelMappingsDraft}
                        models={models}
                        loading={modelLoading}
                        startupModel={claudeModelMappingsDraft.startupModel}
                        subagentModel={claudeModelMappingsDraft.subagentModel}
                        disabled={busy || loading}
                        onChange={(field, value) => editClaudeModelMappings((current) => followClaudeCodeContextWindow(
                          {
                            ...current,
                            [field]: value,
                            [field === 'startupModel' ? 'manageDefaultModel' : 'manageSubagentModel']: value.trim().length > 0,
                          },
                          current.maxContextTokens,
                          true,
                        ))}
                      />
                  {modelError ? (
                    <div className="claude-code-model-error" role="status">
                      <div><strong>{t('agents.model.loadFailed')}</strong><p>{t('agents.claudeCodeRuntime.connectionHint')}</p></div>
                      <button type="button" className="secondary-button" onClick={refreshModels} disabled={modelLoading}>
                        {t('agents.model.refresh')}
                      </button>
                    </div>
                  ) : null}
                  <ClaudeCodeRoleRows routes={claudeModelMappingsDraft} models={claudeMappingModels}
                    loading={modelLoading} error={modelError}
                    disabled={busy || !activeStatus?.installed || !activeStatus.supportedPlatform}
                    onModelChange={selectClaudeModelMapping} onContextChange={changeClaude1mPreference}
                    onRefresh={refreshModels} />
                  <details className="claude-code-context-settings">
                    <summary>{t('agents.claudeCodeRuntime.advancedContext')}</summary>
                    <div className="agent-claude-code-runtime-settings">
                      <div className="agent-claude-code-number-field">
                        <span>{t('agents.claudeCodeRuntime.maxContextTokens')}</span>
                        <input
                          type="number"
                          min={100_000}
                          max={1_000_000}
                          step={1_000}
                          value={claudeModelMappingsDraft.maxContextTokens}
                          onChange={(event) => changeClaudeCodeRuntimeSetting(
                            'maxContextTokens',
                            Math.trunc(event.currentTarget.valueAsNumber || 0),
                          )}
                          disabled={busy || loading || modelLoading}
                          aria-label={t('agents.claudeCodeRuntime.maxContextTokens')}
                          aria-describedby="claude-code-max-context-hint"
                        />
                        <small id="claude-code-max-context-hint">
                          {t('agents.claudeCodeRuntime.maxContextTokensHint')}
                        </small>
                      </div>
                      <label className="agent-claude-code-number-field">
                        <span>{t('agents.claudeCodeRuntime.autoCompactPct')}</span>
                        <div className="agent-claude-code-percent-input">
                          <input
                            type="number"
                            min={1}
                            max={100}
                            step={1}
                            value={claudeModelMappingsDraft.autoCompactPct}
                            onChange={(event) => changeClaudeCodeRuntimeSetting(
                              'autoCompactPct',
                              Math.trunc(event.currentTarget.valueAsNumber || 0),
                            )}
                            disabled={busy || loading || modelLoading || claudeModelMappingsDraft.disableAutoCompact}
                            aria-describedby="claude-code-auto-compact-hint"
                          />
                          <span>%</span>
                        </div>
                        <small id="claude-code-auto-compact-hint">
                          {t('agents.claudeCodeRuntime.autoCompactPctHint')}
                        </small>
                      </label>
                      <label
                        className="agent-claude-code-disable-compact"
                        title={t('agents.claudeCodeRuntime.disableAutoCompactHint')}
                      >
                        <span>
                          <strong>{t('agents.claudeCodeRuntime.disableAutoCompact')}</strong>
                          <small>{t('agents.claudeCodeRuntime.disableAutoCompactHint')}</small>
                        </span>
                        <span className="switch-control">
                          <input
                            type="checkbox"
                            checked={claudeModelMappingsDraft.disableAutoCompact}
                            onChange={(event) => changeClaudeCodeAutoCompactDisabled(
                              event.currentTarget.checked,
                            )}
                            disabled={busy || loading || modelLoading}
                          />
                          <span className="switch-track" />
                        </span>
                      </label>
                    </div>
                  </details>
                </section>
              ) : null}

              <div className="agent-save-bar">
                {configurationFeedback}
                  <div className={`agent-save-actions${!isPiClient ? " agent-codex-save-actions" : ""}`}>
                  {closeConfigurationButton}
                  <button type="button" className="primary-button" onClick={applySelectedConfiguration}
                    disabled={busy || !canEnable || configurationWriteBlocked}
                  >
                    {['apply', 'install-pi', 'repair-pi'].includes(busyAction ?? '') ? <LoaderCircle size={16} className="spin" /> : null}
                    {isPiClient && !activeStatus?.pluginInstalled ? t('agents.pi.install') : configurationActionLabel}
                  </button>
                </div>
              </div>

            </div>
          ) : null}

          {!embedded && selected === 'codex' && activeSubpage === 'sessions' ? (
            <div
              className="agent-sessions-page"
              id="agent-subpage-panel-sessions"
              role="tabpanel"
              aria-labelledby="agent-subpage-tab-sessions"
            >
              <CodexSessionsPanel />
            </div>
          ) : null}
          {activeSubpage === 'management' ? (
            <div id="agent-subpage-panel-management" role="tabpanel" aria-labelledby="agent-subpage-tab-management">
              <AgentConfigManagementPanel pi={isPiClient} codex={selected === 'codex'} busyAction={busyAction}
                executablePath={executablePaths[selected] ?? ''} onChooseExecutablePath={() => void chooseExecutablePath()}
                onClearExecutablePath={() => clearExecutablePath()}
                dualTargets={hasIndependentCliAndApp} desktopExecutablePath={executablePaths[`${selected}:app`] ?? ''}
                onChooseDesktopExecutablePath={() => void chooseExecutablePath(true)}
                onClearDesktopExecutablePath={() => clearExecutablePath(true)}
                canTemplate={canEnable && !nativeOauth} canUpdatePi={canEnable && !configurationWriteBlocked && Boolean(activeStatus?.pluginInstalled)}
                canUninstallPi={launchEnabled && Boolean(activeStatus?.pluginInstalled)}
                pluginInstalled={Boolean(activeStatus?.pluginInstalled)} pluginVersion={activeStatus?.pluginVersion ?? null}
                updateLabel={piPluginUpdateAvailable ? piPluginUpdateTitle ?? '' : ''}
                onBackup={() => void createManualBackup()} onRestore={() => setBackupsOpen(true)}
                onTemplate={() => void openDefaultConfirmation()} onClear={openClearConfirmation}
                canClearIntegration={!loading && Boolean(activeStatus?.configValid && activeStatus.supportedPlatform)}
                onClearIntegration={() => void clearCodexIntegration()}
                onUpdatePi={() => void updatePiProvider()}
                onUninstallPi={() => void uninstallPiProvider()} />
            </div>
          ) : null}
          {activeSubpage !== 'core' ? configurationFeedback : null}
          {selected === 'vscode-copilot' && activeSubpage === 'core' && busyAction === 'apply' ? <p className="agent-model-hint" role="status">{t('agents.copilotSetup.pending')}</p> : null}
          <MessageNotice message={configurationErrorMessage}
            onDismiss={() => { setConfigurationError(''); setModelSelectionError(''); setModelError(''); }} />
          <MessageNotice tone="success" message={!configurationErrorMessage ? configurationNotice || clearNotice : null}
            onDismiss={() => { setConfigurationNotice(''); setClearNotice(''); }} />
          {activeSubpage === 'core' ? <AgentRunControls name={activeDefinition.name} dualTargets={hasIndependentCliAndApp}
            desktop={(hasIndependentCliAndApp && !isDeepSeekHarnessClient) || selected === 'claude-desktop' || selected === 'zcode' || selected === 'workbuddy'}
            cliService={selected === 'codex'} cliServiceRestarting={selected === 'codex' && cliServiceRestarting}
            targets={activeLaunchTargets} enabled={launchEnabled} busyAction={busyAction}
            harness={isDeepSeekHarnessClient ? deepSeekHarnessProcessStatus : null}
            onLaunch={(target) => void launchAgent(target)} onRestart={() => void restartDesktopApp()}
            onRestartCli={() => void restartCodexCliService()}
            onStop={() => void stopDeepSeekHarness()} onRestartWeb={() => void restartDeepSeekHarness()} error={launchError} onErrorDismiss={() => setLaunchError('')} /> : null}
          </div>
        </section>
      </div>

      {desktopHelpOpen ? <ClaudeDesktopHelpDialog onClose={() => setDesktopHelpOpen(false)} /> : null}
      {backupsOpen ? <AgentConfigBackupDialog client={selected} onClose={() => setBackupsOpen(false)} onRestored={async () => {
        clearPendingChanges();
        if (selected === 'claude-code' || selected === 'claude-desktop') claudeModelMappingsDirtyRef.current[selected] = false;
        setOauthConfigurationDraft(null);
        const refreshed = await invoke<AgentConfigStatus[]>('refresh_agent_config_statuses', { executableOverrides: executablePaths });
        setStatuses(refreshed);
        const current = refreshed.find((status) => status.id === selected)?.currentModel;
        setModelByClient((values) => { const next = { ...values, [selected]: current ?? '' }; writeAgentModelSelections(next); return next; });
        setConfigurationNotice(t('agents.backup.restored'));
      }} /> : null}

      {harnessCatalogDialogOpen ? (
        <DeepSeekHarnessCatalogDialog onClose={() => setHarnessCatalogDialogOpen(false)}
          onSaved={async () => { await loadModels('deepseek-harness', selectedModel); await loadStatuses(true); }} />
      ) : null}
      {codexCatalogDialogOpen ? (
        <CodexModelCatalogDialog
          onClose={() => setCodexCatalogDialogOpen(false)}
          onSaved={() => loadModels('codex', selectedModel)}
        />
      ) : null}

      {launchDirectoryDialogOpen ? (
        <div className="config-dialog-backdrop" onMouseDown={(event) => {
          if (event.currentTarget === event.target && !busy) {
            setLaunchDirectoryDialogOpen(false);
            setLaunchDirectoryTarget(null);
          }
        }}>
          <section
            ref={launchDirectoryDialogRef}
            className="config-dialog agent-launch-directory-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="agent-launch-directory-title"
          >
            <div className="config-dialog-heading">
              <div>
                <h2 id="agent-launch-directory-title">
                  {isDeepSeekHarnessClient
                    ? t('agents.deepseekLaunch.dialogTitle')
                    : t('agents.launchDirectory.dialogTitle', { client: activeDefinition.name })}
                </h2>
              </div>
            </div>
            <p>{isDeepSeekHarnessClient
              ? t('agents.deepseekLaunch.dialogDescription')
              : t('agents.launchDirectory.dialogDescription', { client: activeDefinition.name })}</p>
            {isDeepSeekHarnessClient ? (
              <div className="deepseek-harness-launch-options">
                <label className="config-dialog-field deepseek-harness-launch-field">
                  <span>{t('agents.deepseekLaunch.mode.label')}</span>
                  <select
                    className="config-dialog-text-input"
                    value={deepSeekHarnessLaunchDraft.mode}
                    onChange={(event) => updateDeepSeekHarnessLaunchDraft({
                      mode: event.currentTarget.value as DeepSeekHarnessLaunchMode,
                    })}
                    disabled={busy}
                  >
                    <option value="web">{t('agents.deepseekLaunch.mode.web')}</option>
                    <option value="headless">{t('agents.deepseekLaunch.mode.headless')}</option>
                    <option value="acp">{t('agents.deepseekLaunch.mode.acp')}</option>
                    <option value="sdk">{t('agents.deepseekLaunch.mode.sdk')}</option>
                    <option value="sdk-minimal">{t('agents.deepseekLaunch.mode.sdkMinimal')}</option>
                    <option value="custom">{t('agents.deepseekLaunch.mode.custom')}</option>
                  </select>
                </label>
                <p className="deepseek-harness-launch-mode-description">
                  {deepSeekHarnessLaunchModeDescription}
                </p>

                {deepSeekHarnessLaunchDraft.mode === 'web' ? (
                  <>
                    <div className="deepseek-harness-launch-grid">
                      <label className="config-dialog-field deepseek-harness-launch-field">
                        <span>{t('agents.deepseekLaunch.web.host')}</span>
                        <input
                          className="config-dialog-text-input"
                          type="text"
                          value={deepSeekHarnessLaunchDraft.webHost}
                          onChange={(event) => updateDeepSeekHarnessLaunchDraft({ webHost: event.currentTarget.value })}
                          placeholder={t('agents.deepseekLaunch.web.hostPlaceholder')}
                          disabled={busy}
                        />
                      </label>
                      <label className="config-dialog-field deepseek-harness-launch-field">
                        <span>{t('agents.deepseekLaunch.web.port')}</span>
                        <input
                          className="config-dialog-text-input"
                          type="text"
                          inputMode="numeric"
                          value={deepSeekHarnessLaunchDraft.webPort}
                          onChange={(event) => updateDeepSeekHarnessLaunchDraft({ webPort: event.currentTarget.value })}
                          placeholder={t('agents.deepseekLaunch.web.portPlaceholder')}
                          disabled={busy}
                        />
                      </label>
                    </div>
                    <label className="deepseek-harness-launch-checkbox">
                      <input
                        type="checkbox"
                        checked={deepSeekHarnessLaunchDraft.openBrowser}
                        onChange={(event) => updateDeepSeekHarnessLaunchDraft({ openBrowser: event.currentTarget.checked })}
                        disabled={busy}
                      />
                      <span>
                        <strong>{t('agents.deepseekLaunch.web.openBrowser')}</strong>
                        <small>{t('agents.deepseekLaunch.web.openBrowserDescription')}</small>
                      </span>
                    </label>
                    <label className="config-dialog-field deepseek-harness-launch-field multiline">
                      <span>{t('agents.deepseekLaunch.web.trustedHosts')}</span>
                      <textarea
                        className="config-dialog-text-input"
                        value={deepSeekHarnessLaunchDraft.trustedHosts}
                        onChange={(event) => updateDeepSeekHarnessLaunchDraft({ trustedHosts: event.currentTarget.value })}
                        placeholder={t('agents.deepseekLaunch.web.trustedHostsPlaceholder')}
                        disabled={busy}
                      />
                    </label>
                  </>
                ) : null}

                {deepSeekHarnessLaunchDraft.mode === 'headless' ? (
                  <label className="config-dialog-field deepseek-harness-launch-field">
                    <span>{t('agents.deepseekLaunch.headless.task')}</span>
                    <input
                      className="config-dialog-text-input"
                      type="text"
                      value={deepSeekHarnessLaunchDraft.task}
                      onChange={(event) => updateDeepSeekHarnessLaunchDraft({ task: event.currentTarget.value })}
                      placeholder={t('agents.deepseekLaunch.headless.taskPlaceholder')}
                      disabled={busy}
                    />
                  </label>
                ) : null}

                {deepSeekHarnessLaunchDraft.mode === 'custom' ? (
                  <label className="config-dialog-field deepseek-harness-launch-field">
                    <span>{t('agents.deepseekLaunch.custom.profile')}</span>
                    <input
                      className="config-dialog-text-input"
                      type="text"
                      value={deepSeekHarnessLaunchDraft.profile}
                      onChange={(event) => updateDeepSeekHarnessLaunchDraft({ profile: event.currentTarget.value })}
                      placeholder={t('agents.deepseekLaunch.custom.profilePlaceholder')}
                      disabled={busy}
                    />
                  </label>
                ) : null}

                <label className="config-dialog-field deepseek-harness-launch-field multiline">
                  <span>{t('agents.deepseekLaunch.patches')}</span>
                  <textarea
                    className="config-dialog-text-input"
                    value={deepSeekHarnessLaunchDraft.patches}
                    onChange={(event) => updateDeepSeekHarnessLaunchDraft({ patches: event.currentTarget.value })}
                    placeholder={t('agents.deepseekLaunch.patchesPlaceholder')}
                    disabled={busy}
                  />
                </label>
              </div>
            ) : null}
            <div className="config-dialog-field">
              <span>{t('agents.launchDirectory.workingDirectory')}</span>
              <button
                type="button"
                className="agent-launch-directory-picker"
                onClick={() => void chooseLaunchDirectory()}
                disabled={busy}
                autoFocus
              >
                <span>
                  <small>{launchDirectory
                    ? t('agents.launchDirectory.selectedDirectory')
                    : t('agents.launchDirectory.noDirectory')}</small>
                  <strong title={launchDirectory || undefined}>
                    {launchDirectory || t('agents.launchDirectory.chooseDirectory')}
                  </strong>
                </span>
                <b>{busyAction === 'directory'
                  ? t('agents.launchDirectory.choosing')
                  : t('agents.launchDirectory.browse')}</b>
              </button>
            </div>
            {activeLaunchDirectoryHistory.length ? (
              <div className="agent-launch-directory-history">
                <strong>{t('agents.launchDirectory.historyTitle')}</strong>
                <div className="agent-launch-directory-history-list">
                  {activeLaunchDirectoryHistory.map((directory, index) => {
                    const active = directory === launchDirectory;
                    return (
                      <button
                        type="button"
                        className={active ? 'active' : ''}
                        key={directory}
                        onClick={() => {
                          setLaunchDirectory(directory);
                          setLaunchDirectoryError('');
                        }}
                        disabled={busy}
                        title={directory}
                      >
                        <span>
                          <strong>{directory}</strong>
                          <small>{index === 0
                            ? t('agents.launchDirectory.lastUsed')
                            : t('agents.launchDirectory.recentItem')}</small>
                        </span>
                        <b>
                          {active ? <Check size={15} aria-hidden /> : null}
                          {active
                            ? t('agents.launchDirectory.historySelected')
                            : t('agents.launchDirectory.historyUse')}
                        </b>
                      </button>
                    );
                  })}
                </div>
              </div>
            ) : null}
            {launchDirectoryError ? (
              <MessageNotice message={launchDirectoryError} onDismiss={() => setLaunchDirectoryError('')} />
            ) : null}
            <div className="config-dialog-actions two-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => {
                  setLaunchDirectoryDialogOpen(false);
                  setLaunchDirectoryTarget(null);
                }}
                disabled={busy}
              >
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className="primary-button"
                onClick={() => void launchFromDirectoryDialog()}
                disabled={busy || !launchDirectory.trim() || !launchDirectoryTarget}
              >
                {busyAction === 'launch' || busyAction === 'launch-cli'
                  ? <LoaderCircle size={16} className="spin" />
                  : null}
                {isDeepSeekHarnessClient
                  ? t('agents.deepseekLaunch.launch', { mode: deepSeekHarnessLaunchModeLabel })
                  : t('agents.launchDirectory.launch', { client: activeDefinition.name })}
              </button>
            </div>
          </section>
        </div>
      ) : null}

      {defaultConfirmOpen ? (
        <div className="config-dialog-backdrop">
          <section ref={defaultDialogRef} className="config-dialog agent-restore-dialog" role="alertdialog" aria-modal="true" aria-labelledby="agent-default-title">
            <div className="config-dialog-heading">
              <div><AlertTriangle size={19} /><h2 id="agent-default-title">{t('agents.default.title')}</h2></div>
            </div>
            <p>
              {t('agents.default.description', { name: activeDefinition.name })}
            </p>
            {templatePreview ? <ul className="agent-template-files">{templatePreview.files.map((file) => <li key={file}><code>{file}</code></li>)}</ul> : null}
            {defaultError ? (
              <MessageNotice message={defaultError} onDismiss={() => setDefaultError('')} />
            ) : null}
            <div className="config-dialog-actions two-actions">
              <button type="button" className="secondary-button" onClick={closeDefaultConfirmation} disabled={busy}>{t('common.cancel')}</button>
              <button type="button" className="danger-button" onClick={() => void resetConfigurationToDefault()} disabled={busy || !templatePreview}>
                {busyAction === 'default' ? <LoaderCircle size={16} className="spin" /> : null}
                {t('agents.default.confirm')}
              </button>
            </div>
          </section>
        </div>
      ) : null}

      {clearConfirmOpen ? (
        <div className="config-dialog-backdrop">
          <section ref={clearDialogRef} className="config-dialog agent-restore-dialog" role="alertdialog" aria-modal="true" aria-labelledby="agent-clear-title">
            <div className="config-dialog-heading">
              <div><AlertTriangle size={19} /><h2 id="agent-clear-title">{selected === 'codex' ? t('agents.clear.title') : t('agents.clearIntegration.title', { name: activeDefinition.name })}</h2></div>
            </div>
            <p>{t(selected === 'codex' ? 'agents.clear.description' : 'agents.clearIntegration.description')}</p>
            {clearError ? (
              <MessageNotice message={clearError} onDismiss={() => setClearError('')} />
            ) : null}
            <div className="config-dialog-actions two-actions">
              <button type="button" className="secondary-button" onClick={closeClearConfirmation} disabled={busy}>{t('common.cancel')}</button>
              <button type="button" className="danger-button" onClick={() => void clearConfiguration()} disabled={busy}>
                {busyAction === 'clear' ? <LoaderCircle size={16} className="spin" /> : <Trash2 size={16} />}
                {t(selected === 'codex' ? 'agents.clear.confirm' : 'agents.clearIntegration.confirm')}
              </button>
            </div>
          </section>
        </div>
      ) : null}

      {oauthLoginRequiredAction ? (
        <div className="config-dialog-backdrop">
          <section ref={oauthRequiredDialogRef} className="config-dialog agent-restore-dialog" role="alertdialog" aria-modal="true" aria-labelledby="agent-oauth-login-required-title">
            <div className="config-dialog-heading">
              <div><AlertTriangle size={19} /><h2 id="agent-oauth-login-required-title">{t('agents.oauthLoginRequired.title')}</h2></div>
            </div>
            <p>{oauthLoginRequiredDescription}</p>
            <div className={`config-dialog-actions${oauthLoginRequiredAction === 'launch' ? ' single-action' : ''}`}>
              <button type="button" className={oauthLoginRequiredAction === 'launch' ? 'primary-button' : 'secondary-button'}
                onClick={() => setOauthLoginRequiredAction(null)}>{t('agents.oauthLoginRequired.confirm')}</button>
              {oauthLoginRequiredAction !== 'launch' ? (
                <button type="button" className="primary-button" onClick={openClearIntegrationGuide}>
                  {t('agents.oauthLoginRequired.openManagement')}
                </button>
              ) : null}
            </div>
          </section>
        </div>
      ) : null}
      {confirmationDialog}
    </section>
  );
}
