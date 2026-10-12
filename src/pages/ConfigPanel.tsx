import { CoolingToggle } from '../components/settings/CoolingToggle';
import { FormEvent, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AlertCircle,
  Search,
  ChevronRight,
  Puzzle,
  SlidersHorizontal,
  Bug,
  Check,
  Copy,
  Clock3,
  Database,
  Eye,
  EyeOff,
  FileText,
  FolderOpen,
  Gauge,
  HardDrive,
  KeyRound,
  Link2,
  LockKeyhole,
  Network,
  Pencil,
  Plus,
  RefreshCw,
  Route,
  Settings2,
  ShieldCheck,
  Power,
  Terminal,
  Trash2,
  X,
} from 'lucide-react';
import { useCoreRuntime, type CoreStatus } from '../coreRuntime';
import { useI18n } from '../i18n';
import { MessageNotice, FloatingNotice, useAppNotice } from '../appNotice';
import { webUiManagementUrl } from '../services/clientAccess';
import { ThinkingAliasesPage } from './ThinkingAliasesPage';
import { SensitiveWordsPage } from './SensitiveWordsPage';
import { handleHorizontalTabKey } from '../components/tabKeyboardNavigation';
import { useDialogFocusTrap } from '../components/useDialogFocusTrap';
import { useConfirmation } from '../components/ConfirmationDialog';
import { TemplateConfigSection } from '../components/TemplateConfigSection';
import { SettingsHelp } from '../components/SettingsHelp';
import { templateMessages, templateText } from '../i18n/templateConfig';
import {
  settingsCategories, settingsTemplateGroups,
  allSettingsTemplateGroups, settingsMessages, takeRequestedSettingField, type SettingsCategory,
} from '../services/settingsNavigation';

type CoreConfigSettings = {
  managementSecretConfigured: boolean;
  apiKeys: CoreApiKey[];
  debug: boolean;
  commercialMode: boolean;
  loggingToFile: boolean;
  logsMaxTotalSizeMb: number;
  errorLogsMaxFiles: number;
  usageStatisticsEnabled: boolean;
  redisUsageQueueRetentionSeconds: number;
  host: string;
  port: number;
  allowLan: boolean;
  routingStrategy: string;
  proxyUrl: string;
  proxyOverride: boolean;
  routingSessionAffinity: boolean;
  routingSessionAffinityTtl: string;
  disableCooling: boolean;
  requestRetry: number;
  maxRetryCredentials: number;
  maxRetryInterval: number;
  streamingBootstrapRetries: number;
};

type CoreApiKey = {
  apiKey: string;
  remark: string;
};

type ConfigAction =
  | 'add-key'
  | 'update-key'
  | 'delete-key'
  | 'management-secret'
  | 'logging'
  | 'open-logs'
  | 'routing'
  | 'network'
  | 'retry'
  | 'tls'
  | 'software'
  | null;
type ConfigSubpage = SettingsCategory;
const CONFIG_SUBPAGES = settingsCategories.map(category => category.id);
const CATEGORY_ICONS = { general: ShieldCheck, aliases: Link2, routing: Route, requests: SlidersHorizontal, oauth: KeyRound, diagnostics: FileText, extensions: Puzzle, software: Settings2 };
type SettingDestination = { category: ConfigSubpage; target: string; field?: string };
type SettingSearchEntry = SettingDestination & { title: string; context: string; keywords: string };

type CloseBehavior = 'ask' | 'exit' | 'minimize-to-tray';
type NetworkDraftField =
  | 'port'
  | 'host'
  | 'proxyUrl'
  | 'proxyOverride'
  | 'sessionAffinity'
  | 'sessionTtl'
  | 'disableCooling'
  | 'requestRetry'
  | 'maxRetryCredentials'
  | 'maxRetryInterval'
  | 'streamingBootstrapRetries';
type DraftRefreshMode = 'replace' | 'preserve';

type NetworkDraftDirty = Record<NetworkDraftField, boolean>;

type AgentTerminalOption = {
  id: string;
  label: string;
};

type SoftwareSettings = {
  closeBehavior: CloseBehavior;
  autostartEnabled: boolean;
  startCoreOnLaunch: boolean;
  silentStartEnabled: boolean;
  defaultTerminal: string;
  availableTerminals: AgentTerminalOption[];
};

type CoreTlsSettings = {
  enabled: boolean;
  cert: string;
  key: string;
};

const cleanNetworkDraft = (): NetworkDraftDirty => ({
  port: false,
  host: false,
  proxyUrl: false,
  proxyOverride: false,
  sessionAffinity: false,
  sessionTtl: false,
  disableCooling: false,
  requestRetry: false,
  maxRetryCredentials: false,
  maxRetryInterval: false,
  streamingBootstrapRetries: false,
});

const ROUTING_OPTIONS = [
  { value: 'round-robin', labelKey: 'config.routing.roundRobin' },
  { value: 'weighted-round-robin', labelKey: 'config.routing.roundRobin' },
  { value: 'fill-first', labelKey: 'config.routing.fillFirst' },
] as const;

export function ConfigPanelPage() {
  const { t, locale } = useI18n();
  const { askConfirmation, confirmationDialog } = useConfirmation();
  const { status: coreStatus, publishStatus, refreshStatus } = useCoreRuntime();
  const [settings, setSettings] = useState<CoreConfigSettings | null>(null);
  const [softwareSettings, setSoftwareSettings] = useState<SoftwareSettings | null>(null);
  const [softwareSettingsLoading, setSoftwareSettingsLoading] = useState(true);
  const [softwareCloseBehaviorDraft, setSoftwareCloseBehaviorDraft] = useState<CloseBehavior>('ask');
  const [softwareAutostartDraft, setSoftwareAutostartDraft] = useState(false);
  const [softwareStartCoreDraft, setSoftwareStartCoreDraft] = useState(true);
  const [softwareSilentStartDraft, setSoftwareSilentStartDraft] = useState(false);
  const [softwareDefaultTerminalDraft, setSoftwareDefaultTerminalDraft] = useState('auto');
  const [softwareSavedStatusVisible, setSoftwareSavedStatusVisible] = useState(false);
  const [tlsSettings, setTlsSettings] = useState<CoreTlsSettings | null>(null);
  const [tlsSettingsLoading, setTlsSettingsLoading] = useState(true);
  const [tlsEnabledDraft, setTlsEnabledDraft] = useState(false);
  const [tlsCertDraft, setTlsCertDraft] = useState('');
  const [tlsKeyDraft, setTlsKeyDraft] = useState('');
  const [tlsError, setTlsError] = useState('');
  const [tlsFileSelecting, setTlsFileSelecting] = useState<'cert' | 'key' | null>(null);
  const [tlsSavedStatusVisible, setTlsSavedStatusVisible] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState('');
  const [busyAction, setBusyAction] = useState<ConfigAction>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [editingApiKey, setEditingApiKey] = useState<string | null>(null);
  const [deleteIndex, setDeleteIndex] = useState<number | null>(null);
  const [newApiKey, setNewApiKey] = useState('');
  const [newApiKeyRemark, setNewApiKeyRemark] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [formError, setFormError] = useState('');
  const [managementSecretDraft, setManagementSecretDraft] = useState('');
  const [managementSecretConfirm, setManagementSecretConfirm] = useState('');
  const [showManagementSecret, setShowManagementSecret] = useState(false);
  const [managementSecretError, setManagementSecretError] = useState('');
  const [debugDraft, setDebugDraft] = useState(false);
  const [commercialModeDraft, setCommercialModeDraft] = useState(false);
  const [loggingToFileDraft, setLoggingToFileDraft] = useState(false);
  const [logsMaxTotalSizeDraft, setLogsMaxTotalSizeDraft] = useState('0');
  const [errorLogsMaxFilesDraft, setErrorLogsMaxFilesDraft] = useState('10');
  const [usageStatisticsDraft, setUsageStatisticsDraft] = useState(true);
  const [redisUsageRetentionDraft, setRedisUsageRetentionDraft] = useState('60');
  const [loggingError, setLoggingError] = useState('');
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const keyFeedback = useAppNotice();
  const managementFeedback = useAppNotice();
  const loggingFeedback = useAppNotice();
  const networkFeedback = useAppNotice();
  const routingFeedback = useAppNotice();
  const retryFeedback = useAppNotice();
  const tlsFeedback = useAppNotice();
  const softwareFeedback = useAppNotice();
  const renderFeedback = (feedback: ReturnType<typeof useAppNotice>) => (
    <FloatingNotice key={feedback.revision} notice={feedback.notice} onDismiss={feedback.clearNotice} />
  );
  const [activeSubpage, setActiveSubpage] = useState<ConfigSubpage>('general');
  const [settingsSearch, setSettingsSearch] = useState('');
  const [dirtyTemplateGroups, setDirtyTemplateGroups] = useState<readonly string[]>([]);
  const [sensitiveWordsDirty, setSensitiveWordsDirty] = useState(false);
  const [sensitiveWordsVisited, setSensitiveWordsVisited] = useState(false);
  const [aliasesVisited, setAliasesVisited] = useState(false);
  const [pendingDestination, setPendingDestination] = useState<SettingDestination | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const categoryNavigationRef = useRef<HTMLDivElement>(null);
  const st = (key: keyof typeof settingsMessages) => templateText(settingsMessages[key], locale);
  const searching = settingsSearch.trim().length > 0;

  useEffect(() => {
    const navigation = categoryNavigationRef.current;
    const tab = document.getElementById(`config-subpage-tab-${activeSubpage}`);
    if (!navigation || !tab) return;
    const revealSelectedTab = () => {
      const bounds = navigation.getBoundingClientRect();
      const tabBounds = tab.getBoundingClientRect();
      if (tabBounds.left < bounds.left) navigation.scrollLeft += tabBounds.left - bounds.left;
      else if (tabBounds.right > bounds.right) navigation.scrollLeft += tabBounds.right - bounds.right;
    };
    revealSelectedTab();
    const observer = new ResizeObserver(revealSelectedTab);
    observer.observe(navigation);
    return () => observer.disconnect();
  }, [activeSubpage]);

  useEffect(() => {
    if (!pendingDestination) return;
    const frame = window.requestAnimationFrame(() => {
      const target = document.getElementById(pendingDestination.field ?? pendingDestination.target)
        ?? document.getElementById(pendingDestination.target);
      if (target) {
        const details = target.closest<HTMLDetailsElement>('.template-config-custom') ?? target.querySelector<HTMLDetailsElement>('.template-config-custom');
        if (details) details.open = true;
        target.scrollIntoView({ block: 'center', behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
        const control = target.querySelector<HTMLElement>('input:not(:disabled), select:not(:disabled), textarea:not(:disabled)')
          ?? target.querySelector<HTMLElement>('.template-config-custom > summary');
        (control ?? target).focus({ preventScroll: true });
      }
      setPendingDestination(null);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [pendingDestination]);
  const [portDraft, setPortDraft] = useState('8317');
  const [hostDraft, setHostDraft] = useState('127.0.0.1');
  const [proxyUrlDraft, setProxyUrlDraft] = useState('');
  const [proxyOverrideDraft, setProxyOverrideDraft] = useState(false);
  const [sessionAffinityDraft, setSessionAffinityDraft] = useState(false);
  const [sessionTtlDraft, setSessionTtlDraft] = useState('');
  const [disableCoolingDraft, setDisableCoolingDraft] = useState(true);
  const [requestRetryDraft, setRequestRetryDraft] = useState('3');
  const [maxRetryCredentialsDraft, setMaxRetryCredentialsDraft] = useState('0');
  const [maxRetryIntervalDraft, setMaxRetryIntervalDraft] = useState('30');
  const [streamingBootstrapRetriesDraft, setStreamingBootstrapRetriesDraft] = useState('0');
  const [portError, setPortError] = useState('');
  const [hostError, setHostError] = useState('');
  const [retryError, setRetryError] = useState('');
  const networkDraftDirtyRef = useRef<NetworkDraftDirty>(cleanNetworkDraft());
  const loggingDraftDirtyRef = useRef(false);
  const otherDraftDirtyRef = useRef({ software: false, tls: false });
  const copyTimerRef = useRef<number | null>(null);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | null = null;
    void loadSettings();
    void loadSoftwareSettings();
    void loadTlsSettings();
    void listen('config-files-changed', () => {
      if (!disposed) {
        void loadSettings('preserve');
        void loadSoftwareSettings('preserve');
        void loadTlsSettings('preserve');
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  const applySettings = (result: CoreConfigSettings, mode: DraftRefreshMode = 'replace') => {
    setSettings(result);
    if (mode === 'preserve') {
      const dirty = networkDraftDirtyRef.current;
      if (!dirty.port) setPortDraft(String(result.port));
      if (!dirty.host) setHostDraft(result.host);
      if (!dirty.proxyUrl) setProxyUrlDraft(result.proxyUrl);
      if (!dirty.proxyOverride) setProxyOverrideDraft(result.proxyOverride);
      if (!dirty.sessionAffinity) setSessionAffinityDraft(result.routingSessionAffinity);
      if (!dirty.sessionTtl) setSessionTtlDraft(result.routingSessionAffinityTtl);
      if (!dirty.disableCooling) setDisableCoolingDraft(result.disableCooling);
      if (!dirty.requestRetry) setRequestRetryDraft(String(result.requestRetry));
      if (!dirty.maxRetryCredentials) setMaxRetryCredentialsDraft(String(result.maxRetryCredentials));
      if (!dirty.maxRetryInterval) setMaxRetryIntervalDraft(String(result.maxRetryInterval));
      if (!dirty.streamingBootstrapRetries) {
        setStreamingBootstrapRetriesDraft(String(result.streamingBootstrapRetries));
      }
      if (!loggingDraftDirtyRef.current) {
        setDebugDraft(result.debug);
        setCommercialModeDraft(result.commercialMode);
        setLoggingToFileDraft(result.loggingToFile);
        setLogsMaxTotalSizeDraft(String(result.logsMaxTotalSizeMb));
        setErrorLogsMaxFilesDraft(String(result.errorLogsMaxFiles));
        setUsageStatisticsDraft(result.usageStatisticsEnabled);
        setRedisUsageRetentionDraft(String(result.redisUsageQueueRetentionSeconds));
      }
      return;
    }
    networkDraftDirtyRef.current = cleanNetworkDraft();
    loggingDraftDirtyRef.current = false;
    setPortDraft(String(result.port));
    setHostDraft(result.host);
    setProxyUrlDraft(result.proxyUrl);
    setProxyOverrideDraft(result.proxyOverride);
    setSessionAffinityDraft(result.routingSessionAffinity);
    setSessionTtlDraft(result.routingSessionAffinityTtl);
    setDisableCoolingDraft(result.disableCooling);
    setRequestRetryDraft(String(result.requestRetry));
    setMaxRetryCredentialsDraft(String(result.maxRetryCredentials));
    setMaxRetryIntervalDraft(String(result.maxRetryInterval));
    setStreamingBootstrapRetriesDraft(String(result.streamingBootstrapRetries));
    setDebugDraft(result.debug);
    setCommercialModeDraft(result.commercialMode);
    setLoggingToFileDraft(result.loggingToFile);
    setLogsMaxTotalSizeDraft(String(result.logsMaxTotalSizeMb));
    setErrorLogsMaxFilesDraft(String(result.errorLogsMaxFiles));
    setUsageStatisticsDraft(result.usageStatisticsEnabled);
    setRedisUsageRetentionDraft(String(result.redisUsageQueueRetentionSeconds));
    setPortError('');
    setHostError('');
    setRetryError('');
    setLoggingError('');
  };

  const markLoggingDraftDirty = () => {
    loggingDraftDirtyRef.current = true;
    setLoggingError('');
  };

  const markDraftDirty = (field: NetworkDraftField) => {
    networkDraftDirtyRef.current[field] = true;
  };

  const clearDraftDirty = (field: NetworkDraftField) => {
    networkDraftDirtyRef.current[field] = false;
  };

  async function loadSettings(mode: DraftRefreshMode = 'replace') {
    setLoading(true);
    setLoadError('');
    try {
      const result = await invoke<CoreConfigSettings>('get_core_config_settings');
      applySettings(result, mode);
    } catch (error) {
      setSettings(null);
      setLoadError(String(error));
    } finally {
      setLoading(false);
    }
  }

  async function loadSoftwareSettings(mode: DraftRefreshMode = 'replace') {
    setSoftwareSettingsLoading(true);
    setSoftwareSavedStatusVisible(false);
    try {
      const result = await invoke<SoftwareSettings>('get_software_settings');
      if (mode === 'preserve' && otherDraftDirtyRef.current.software) return;
      setSoftwareSettings(result);
      setSoftwareCloseBehaviorDraft(result.closeBehavior);
      setSoftwareAutostartDraft(result.autostartEnabled);
      setSoftwareStartCoreDraft(result.startCoreOnLaunch);
      setSoftwareSilentStartDraft(result.silentStartEnabled);
      setSoftwareDefaultTerminalDraft(result.defaultTerminal);
    } catch (error) {
      setSoftwareSettings(null);
      softwareFeedback.showNotice({ key: 'config.error.saveFailed', variables: { error: String(error) } }, 'error');
    } finally {
      setSoftwareSettingsLoading(false);
    }
  }

  async function loadTlsSettings(mode: DraftRefreshMode = 'replace') {
    setTlsSettingsLoading(true);
    try {
      const result = await invoke<CoreTlsSettings>('get_core_tls_settings');
      if (mode === 'preserve' && otherDraftDirtyRef.current.tls) return;
      setTlsSettings(result);
      setTlsEnabledDraft(result.enabled);
      setTlsCertDraft(result.cert);
      setTlsKeyDraft(result.key);
      setTlsError('');
    } catch (error) {
      setTlsSettings(null);
      setTlsError(String(error));
    } finally {
      setTlsSettingsLoading(false);
    }
  }

  const runMutation = async (
    action: Exclude<ConfigAction, null>,
    command: string,
    args: Record<string, unknown>,
    successMessage: string,
  ) => {
    setBusyAction(action);
    const mutationFeedback = action === 'management-secret'
      ? managementFeedback
      : action === 'routing' ? routingFeedback : keyFeedback;
    mutationFeedback.clearNotice();
    try {
      const result = await invoke<CoreConfigSettings>(command, args);
      setSettings(result);
      setLoadError('');
      if (action !== 'routing') mutationFeedback.showNotice(successMessage, 'success');
      return true;
    } catch (error) {
      if (settings) setSettings(settings);
      mutationFeedback.showNotice({ key: 'config.error.saveFailed', variables: { error: String(error) } }, 'error');
      void loadSettings('preserve');
      return false;
    } finally {
      setBusyAction(null);
    }
  };

  const openAddDialog = () => {
    keyFeedback.clearNotice();
    setEditingApiKey(null);
    setNewApiKey('');
    setNewApiKeyRemark('');
    setShowApiKey(false);
    setFormError('');
    setAddDialogOpen(true);
  };

  const openEditDialog = (entry: CoreApiKey) => {
    keyFeedback.clearNotice();
    setEditingApiKey(entry.apiKey);
    setNewApiKey(entry.apiKey);
    setNewApiKeyRemark(entry.remark);
    setShowApiKey(false);
    setFormError('');
    setAddDialogOpen(true);
  };

  const closeAddDialog = () => {
    if (busyAction === 'add-key' || busyAction === 'update-key') {
      return;
    }
    setAddDialogOpen(false);
    setEditingApiKey(null);
    setFormError('');
  };

  const generateApiKey = () => {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    const value = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
    setNewApiKey(`sk-${value}`);
    setShowApiKey(true);
    setFormError('');
  };

  const generateManagementSecret = () => {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    const value = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
    const secret = `webui-${value}`;
    setManagementSecretDraft(secret);
    setManagementSecretConfirm(secret);
    setShowManagementSecret(true);
    setManagementSecretError('');
  };

  const saveManagementSecret = async (event: FormEvent) => {
    event.preventDefault();
    const secretKey = managementSecretDraft.trim();
    if (!secretKey) {
      setManagementSecretError(t('config.webuiKey.error.empty'));
      return;
    }
    if (secretKey.length > 512) {
      setManagementSecretError(t('config.webuiKey.error.tooLong'));
      return;
    }
    if (/[\u0000-\u001f\u007f-\u009f]/.test(secretKey)) {
      setManagementSecretError(t('config.webuiKey.error.invalid'));
      return;
    }
    if (secretKey !== managementSecretConfirm.trim()) {
      setManagementSecretError(t('config.webuiKey.error.mismatch'));
      return;
    }

    const saved = await runMutation(
      'management-secret',
      'set_core_management_secret_key',
      { secretKey },
      t('config.webuiKey.notice.updated'),
    );
    if (saved) {
      setManagementSecretDraft('');
      setManagementSecretConfirm('');
      setShowManagementSecret(false);
      setManagementSecretError('');
    }
  };

  const disableManagement = async () => {
    const confirmed = await askConfirmation({
      title: templateText(templateMessages.disableManagement, locale),
      message: templateText(templateMessages.disableManagementMessage, locale),
      confirmText: templateText(templateMessages.disableManagement, locale),
      variant: 'danger',
    });
    if (!confirmed) return;
    const saved = await runMutation('management-secret', 'clear_core_management_secret_key', {}, templateText(templateMessages.managementDisabled, locale));
    if (saved) { setManagementSecretDraft(''); setManagementSecretConfirm(''); setShowManagementSecret(false); setManagementSecretError(''); }
  };

  const submitApiKey = async (event: FormEvent) => {
    event.preventDefault();
    const apiKey = newApiKey.trim();
    if (!apiKey) {
      setFormError(t('config.error.emptyKey'));
      return;
    }
    if (!/^[\x21-\x7e]+$/.test(apiKey)) {
      setFormError(t('config.error.invalidKey'));
      return;
    }
    if (settings?.apiKeys.some((entry) => entry.apiKey === apiKey && entry.apiKey !== editingApiKey)) {
      setFormError(t('config.error.duplicateKey'));
      return;
    }
    const remark = newApiKeyRemark.trim();
    if (remark.length > 80) {
      setFormError(t('config.error.remarkTooLong'));
      return;
    }

    const editing = editingApiKey !== null;
    const saved = await runMutation(
      editing ? 'update-key' : 'add-key',
      editing ? 'update_core_api_key' : 'add_core_api_key',
      editing ? { originalApiKey: editingApiKey, apiKey, remark } : { apiKey, remark },
      editing ? t('config.notice.keyUpdated') : t('config.notice.keyAdded'),
    );
    if (saved) {
      setAddDialogOpen(false);
      setEditingApiKey(null);
      setNewApiKey('');
      setNewApiKeyRemark('');
    }
  };

  const saveCoreLoggingSettings = async () => {
    if (!settings || busyAction !== null) return;
    loggingFeedback.clearNotice();
    const logsMaxTotalSizeMb = Number(logsMaxTotalSizeDraft);
    const errorLogsMaxFiles = Number(errorLogsMaxFilesDraft);
    const redisUsageQueueRetentionSeconds = Number(redisUsageRetentionDraft);
    if (
      !Number.isInteger(logsMaxTotalSizeMb)
      || logsMaxTotalSizeMb < 0
      || logsMaxTotalSizeMb > 4294967295
      || !Number.isInteger(errorLogsMaxFiles)
      || errorLogsMaxFiles < 0
      || errorLogsMaxFiles > 4294967295
    ) {
      const message = t('config.diagnostics.error.nonNegativeInteger');
      setLoggingError(message);
      return;
    }
    if (
      !Number.isInteger(redisUsageQueueRetentionSeconds)
      || redisUsageQueueRetentionSeconds < 1
      || redisUsageQueueRetentionSeconds > 3600
    ) {
      const message = t('config.diagnostics.error.redisRetention');
      setLoggingError(message);
      return;
    }

    const commercialModeChanged = commercialModeDraft !== settings.commercialMode;
    setBusyAction('logging');
    setLoggingError('');
    try {
      const result = await invoke<CoreConfigSettings>('save_core_logging_settings', {
        settings: {
          debug: debugDraft,
          commercialMode: commercialModeDraft,
          loggingToFile: loggingToFileDraft,
          logsMaxTotalSizeMb,
          errorLogsMaxFiles,
          usageStatisticsEnabled: usageStatisticsDraft,
          redisUsageQueueRetentionSeconds,
        },
      });
      loggingDraftDirtyRef.current = false;
      applySettings(result, 'preserve');
      if (commercialModeChanged && coreStatus?.running) {
        const status = await invoke<CoreStatus>('restart_core_process');
        publishStatus(status);
        loggingFeedback.showNotice({ key: 'config.diagnostics.notice.savedAndRestarted' }, 'success');
      } else {
        loggingFeedback.showNotice({ key: 'config.diagnostics.notice.saved' }, 'success');
      }
    } catch (error) {
      const message = t('config.error.saveFailed', { error: String(error) });
      setLoggingError(message);
      void refreshStatus();
      void loadSettings('preserve');
    } finally {
      setBusyAction(null);
    }
  };

  const openCoreLogsDirectory = async () => {
    if (busyAction !== null) return;
    setBusyAction('open-logs');
    try {
      await invoke<void>('open_core_logs_directory');
    } catch (error) {
      loggingFeedback.showNotice({ key: 'config.diagnostics.error.openLogs', variables: { error: String(error) } }, 'error');
    } finally {
      setBusyAction(null);
    }
  };

  const confirmDelete = async () => {
    if (deleteIndex === null) {
      return;
    }
    const deleted = await runMutation(
      'delete-key',
      'delete_core_api_key',
      { apiKey: selectedDeleteKey },
      t('config.notice.keyDeleted'),
    );
    if (deleted) {
      setDeleteIndex(null);
    }
  };

  const copyApiKey = async (apiKey: string, index: number) => {
    keyFeedback.clearNotice();
    try {
      await navigator.clipboard.writeText(apiKey);
      setCopiedIndex(index);
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
      copyTimerRef.current = window.setTimeout(() => {
        setCopiedIndex(null);
        copyTimerRef.current = null;
      }, 1800);
    } catch {
      keyFeedback.showNotice({ key: 'config.notice.keyCopyFailed' }, 'error');
    }
  };

  const openWebUi = async () => {
    try {
      const [latestSettings, latestTlsSettings] = await Promise.all([
        invoke<CoreConfigSettings>('get_core_config_settings'),
        invoke<CoreTlsSettings>('get_core_tls_settings'),
      ]);
      applySettings(latestSettings, 'preserve');
      setTlsSettings(latestTlsSettings);
      if (!otherDraftDirtyRef.current.tls) {
        setTlsEnabledDraft(latestTlsSettings.enabled);
        setTlsCertDraft(latestTlsSettings.cert);
        setTlsKeyDraft(latestTlsSettings.key);
      }
      await invoke('open_external_url', {
        url: webUiManagementUrl(latestSettings.port, latestTlsSettings.enabled, latestSettings.host),
      });
    } catch (error) {
      managementFeedback.showNotice({ key: 'config.webuiKey.error.openFailed', variables: { error: String(error) } }, 'error');
    }
  };

  const saveTlsSettings = async () => {
    tlsFeedback.clearNotice();
    if (tlsSettings === null || busyAction !== null) return;
    const cert = tlsCertDraft.trim();
    const key = tlsKeyDraft.trim();
    if (tlsEnabledDraft && (!cert || !key)) {
      setTlsError(t('config.tls.error.pathsRequired'));
      return;
    }

    setBusyAction('tls');
    setTlsError('');
    setTlsSavedStatusVisible(false);
    try {
      const result = await invoke<CoreTlsSettings>('save_core_tls_settings', {
        settings: { enabled: tlsEnabledDraft, cert, key },
      });
      setTlsSettings(result);
      setTlsEnabledDraft(result.enabled);
      setTlsCertDraft(result.cert);
      setTlsKeyDraft(result.key);
      if (coreStatus?.running) {
        const status = await invoke<CoreStatus>('restart_core_process');
        publishStatus(status);
        tlsFeedback.showNotice({ key: 'config.tls.notice.savedAndRestarted' }, 'success');
      } else {
        tlsFeedback.showNotice({ key: 'config.tls.notice.saved' }, 'success');
      }
      setTlsSavedStatusVisible(true);
    } catch (error) {
      setTlsError(String(error));
      void refreshStatus();
      void loadTlsSettings('preserve');
    } finally {
      setBusyAction(null);
    }
  };

  const selectTlsFile = async (target: 'cert' | 'key') => {
    if (tlsSettings === null || busyAction !== null || tlsFileSelecting !== null) return;
    setTlsFileSelecting(target);
    setTlsError('');
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: target === 'cert'
          ? t('config.tls.selectCertTitle')
          : t('config.tls.selectKeyTitle'),
        filters: [{
          name: target === 'cert' ? t('config.tls.certFile') : t('config.tls.keyFile'),
          extensions: target === 'cert' ? ['pem', 'crt', 'cer'] : ['pem', 'key'],
        }],
      });
      if (typeof selected !== 'string') return;
      setTlsSavedStatusVisible(false);
      if (target === 'cert') setTlsCertDraft(selected);
      else setTlsKeyDraft(selected);
    } catch (error) {
      const message = t('config.tls.error.selectFileFailed', { error: String(error) });
      setTlsError(message);
    } finally {
      setTlsFileSelecting(null);
    }
  };

  const changeRoutingStrategy = async (strategy: string) => {
    if (strategy === settings?.routingStrategy) {
      return;
    }
    await runMutation(
      'routing',
      'set_core_routing_strategy',
      { strategy },
      t('config.notice.routingUpdated'),
    );
  };

  const saveNetworkEndpointSettings = async () => {
    networkFeedback.clearNotice();
    if (!settings || busyAction !== null) return;
    const host = hostDraft.trim();
    const port = Number(portDraft);
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setPortError(t('config.error.portRange'));
      networkFeedback.showNotice({ key: 'config.error.portRange' }, 'error');
      return;
    }

    const proxyUrl = proxyUrlDraft.trim();
    const networkChanged = port !== settings.port || host !== settings.host;
    setHostError('');
    setPortError('');
    setBusyAction('network');
    try {
      const result = await invoke<CoreConfigSettings>('save_network_endpoint_settings', {
        settings: networkDraftDirtyRef.current.proxyUrl || networkDraftDirtyRef.current.proxyOverride
          ? { host, port, proxyUrl, proxyOverride: proxyOverrideDraft }
          : { host, port },
      });
      clearDraftDirty('host');
      clearDraftDirty('port');
      clearDraftDirty('proxyUrl');
      clearDraftDirty('proxyOverride');
      applySettings(result, 'preserve');
      setLoadError('');

      if (networkChanged && coreStatus?.running) {
        try {
          const status = await invoke<CoreStatus>('restart_core_process');
          publishStatus(status);
          networkFeedback.showNotice({ key: 'config.notice.networkRestarted' }, 'success');
        } catch (error) {
          await refreshStatus();
          networkFeedback.showNotice({ key: 'config.error.networkRestartFailed', variables: { error: String(error) } }, 'error');
        }
      } else if (networkChanged) {
        networkFeedback.showNotice({ key: 'config.notice.networkNextStart' }, 'success');
      } else {
        networkFeedback.showNotice({ key: 'config.notice.networkUpdated' }, 'success');
      }
    } catch (error) {
      networkFeedback.showNotice({ key: 'config.error.saveFailed', variables: { error: String(error) } }, 'error');
      void loadSettings('preserve');
    } finally {
      setBusyAction(null);
    }
  };

  const saveRetrySettings = async () => {
    retryFeedback.clearNotice();
    if (!settings || busyAction !== null) return;
    const retryDrafts = [
      requestRetryDraft,
      maxRetryCredentialsDraft,
      maxRetryIntervalDraft,
      streamingBootstrapRetriesDraft,
    ];
    const retryValues = retryDrafts.map(Number);
    if (
      retryDrafts.some((value) => value.length === 0)
      || retryValues.some((value, index) => !Number.isInteger(value) || value < (index === 2 ? -2147483648 : 0) || value > (index === 2 ? 2147483647 : 4294967295))
    ) {
      setRetryError(templateText(templateMessages.retryRange, locale));
      retryFeedback.showNotice(templateText(templateMessages.retryRange, locale), 'error');
      return;
    }
    const [requestRetry, maxRetryCredentials, maxRetryInterval, streamingBootstrapRetries] = retryValues;
    setRetryError('');
    setBusyAction('retry');
    try {
      const result = await invoke<CoreConfigSettings>('save_retry_settings', {
        settings: {
          disableCooling: disableCoolingDraft,
          requestRetry,
          maxRetryCredentials,
          maxRetryInterval,
          streamingBootstrapRetries,
        },
      });
      clearDraftDirty('disableCooling');
      clearDraftDirty('requestRetry');
      clearDraftDirty('maxRetryCredentials');
      clearDraftDirty('maxRetryInterval');
      clearDraftDirty('streamingBootstrapRetries');
      applySettings(result, 'preserve');
      setLoadError('');
      retryFeedback.showNotice({ key: 'config.notice.retryUpdated' }, 'success');
    } catch (error) {
      retryFeedback.showNotice({ key: 'config.error.saveFailed', variables: { error: String(error) } }, 'error');
      void loadSettings('preserve');
    } finally {
      setBusyAction(null);
    }
  };

  const saveSessionRoutingSettings = async () => {
    routingFeedback.clearNotice();
    if (!settings || busyAction !== null) return;
    const routingSessionAffinityTtl = sessionTtlDraft.trim();
    setBusyAction('routing');
    try {
      const result = await invoke<CoreConfigSettings>('save_session_routing_settings', {
        settings: {
          routingSessionAffinity: sessionAffinityDraft,
          routingSessionAffinityTtl,
        },
      });
      clearDraftDirty('sessionAffinity');
      clearDraftDirty('sessionTtl');
      applySettings(result, 'preserve');
      setLoadError('');
      routingFeedback.showNotice({ key: 'config.notice.sessionRoutingUpdated' }, 'success');
    } catch (error) {
      routingFeedback.showNotice({ key: 'config.error.saveFailed', variables: { error: String(error) } }, 'error');
      void loadSettings('preserve');
    } finally {
      setBusyAction(null);
    }
  };

  const saveSoftwareSettings = async () => {
    softwareFeedback.clearNotice();
    if (!softwareSettings || busyAction !== null) return;
    if (
      softwareCloseBehaviorDraft === softwareSettings.closeBehavior
      && softwareAutostartDraft === softwareSettings.autostartEnabled
      && softwareStartCoreDraft === softwareSettings.startCoreOnLaunch
      && softwareSilentStartDraft === softwareSettings.silentStartEnabled
      && softwareDefaultTerminalDraft === softwareSettings.defaultTerminal
    ) return;

    setBusyAction('software');
    try {
      const result = await invoke<SoftwareSettings>('save_software_settings', {
        settings: {
          closeBehavior: softwareCloseBehaviorDraft,
          autostartEnabled: softwareAutostartDraft,
          startCoreOnLaunch: softwareStartCoreDraft,
          silentStartEnabled: softwareSilentStartDraft,
          defaultTerminal: softwareDefaultTerminalDraft,
        },
      });
      setSoftwareSettings(result);
      setSoftwareCloseBehaviorDraft(result.closeBehavior);
      setSoftwareAutostartDraft(result.autostartEnabled);
      setSoftwareStartCoreDraft(result.startCoreOnLaunch);
      setSoftwareSilentStartDraft(result.silentStartEnabled);
      setSoftwareDefaultTerminalDraft(result.defaultTerminal);
      setSoftwareSavedStatusVisible(true);
    } catch (error) {
      setSoftwareSavedStatusVisible(false);
      softwareFeedback.showNotice({ key: 'config.error.saveFailed', variables: { error: String(error) } }, 'error');
      void loadSoftwareSettings('preserve');
    } finally {
      setBusyAction(null);
    }
  };

  const controlsDisabled = loading || settings === null || busyAction !== null;
  const networkSettingsDirty = Boolean(settings) && (
    hostDraft.trim() !== settings?.host
    || portDraft !== String(settings?.port)
    || proxyUrlDraft.trim() !== settings?.proxyUrl
    || proxyOverrideDraft !== settings?.proxyOverride
  );
  const sessionRoutingDirty = Boolean(settings) && (
    sessionAffinityDraft !== settings?.routingSessionAffinity
    || sessionTtlDraft.trim() !== settings?.routingSessionAffinityTtl
  );
  const retrySettingsDirty = Boolean(settings) && (
    disableCoolingDraft !== settings?.disableCooling
    || requestRetryDraft !== String(settings?.requestRetry)
    || maxRetryCredentialsDraft !== String(settings?.maxRetryCredentials)
    || maxRetryIntervalDraft !== String(settings?.maxRetryInterval)
    || streamingBootstrapRetriesDraft !== String(settings?.streamingBootstrapRetries)
  );
  const loggingSettingsDirty = Boolean(settings) && (
    debugDraft !== settings?.debug
    || commercialModeDraft !== settings?.commercialMode
    || loggingToFileDraft !== settings?.loggingToFile
    || logsMaxTotalSizeDraft !== String(settings?.logsMaxTotalSizeMb)
    || errorLogsMaxFilesDraft !== String(settings?.errorLogsMaxFiles)
    || usageStatisticsDraft !== settings?.usageStatisticsEnabled
    || redisUsageRetentionDraft !== String(settings?.redisUsageQueueRetentionSeconds)
  );
  const softwareCloseBehaviorDirty = softwareSettings !== null
    && softwareCloseBehaviorDraft !== softwareSettings.closeBehavior;
  const softwareAutostartDirty = softwareSettings !== null
    && softwareAutostartDraft !== softwareSettings.autostartEnabled;
  const softwareSilentStartDirty = softwareSettings !== null
    && softwareSilentStartDraft !== softwareSettings.silentStartEnabled;
  const softwareStartCoreDirty = softwareSettings !== null
    && softwareStartCoreDraft !== softwareSettings.startCoreOnLaunch;
  const softwareDefaultTerminalDirty = softwareSettings !== null
    && softwareDefaultTerminalDraft !== softwareSettings.defaultTerminal;
  const softwareSettingsDirty = softwareCloseBehaviorDirty
    || softwareAutostartDirty
    || softwareStartCoreDirty
    || softwareSilentStartDirty
    || softwareDefaultTerminalDirty;
  const softwareStatusLabel = softwareSettingsLoading
    ? t('common.loading')
    : softwareSettings === null
      ? t('common.unavailable')
      : softwareSettingsDirty
        ? t('config.network.unsaved')
        : softwareSavedStatusVisible
          ? t('config.network.saved')
          : '';
  const softwareStatusIsSaved = !softwareSettingsLoading
    && softwareSettings !== null
    && !softwareSettingsDirty
    && softwareSavedStatusVisible;
  const tlsSettingsDirty = tlsSettings !== null && (
    tlsEnabledDraft !== tlsSettings.enabled
    || tlsCertDraft.trim() !== tlsSettings.cert
    || tlsKeyDraft.trim() !== tlsSettings.key
  );
  otherDraftDirtyRef.current = { software: softwareSettingsDirty, tls: tlsSettingsDirty };
  const tlsStatusLabel = tlsSettingsLoading
    ? t('common.loading')
    : tlsSettings === null
      ? t('common.unavailable')
      : tlsSettingsDirty
        ? t('config.network.unsaved')
        : tlsSavedStatusVisible
          ? t('config.network.saved')
          : '';
  const tlsStatusIsSaved = !tlsSettingsLoading
    && tlsSettings !== null
    && !tlsSettingsDirty
    && tlsSavedStatusVisible;
  const selectedDeleteKey =
    deleteIndex === null ? '' : settings?.apiKeys[deleteIndex]?.apiKey || '';
  const deletingLastKey = deleteIndex !== null && settings?.apiKeys.length === 1;
  const keyMutationBusy = busyAction === 'add-key' || busyAction === 'update-key';
  const managementSecretBusy = busyAction === 'management-secret';
  const loggingSettingsBusy = busyAction === 'logging';
  const addDialogRef = useDialogFocusTrap<HTMLFormElement>({
    active: addDialogOpen,
    onEscape: keyMutationBusy ? undefined : closeAddDialog,
    preventEscape: keyMutationBusy,
  });
  const deleteDialogRef = useDialogFocusTrap<HTMLDivElement>({
    active: deleteIndex !== null,
    onEscape: busyAction === 'delete-key' ? undefined : () => setDeleteIndex(null),
    preventEscape: busyAction === 'delete-key',
  });
  const activateConfigSubpage = (subpage: ConfigSubpage) => {
    setSettingsSearch('');
    setActiveSubpage(subpage);
    if (subpage === 'requests') {
      setSensitiveWordsVisited(true);
    }
    if (subpage === 'aliases') setAliasesVisited(true);
  };
  const handleConfigTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, subpage: ConfigSubpage) => {
    handleHorizontalTabKey(event, CONFIG_SUBPAGES, subpage, activateConfigSubpage,
      next => document.getElementById(`config-subpage-tab-${next}`));
  };
  const navigateToSetting = (destination: SettingDestination) => {
    activateConfigSubpage(destination.category);
    setPendingDestination(destination);
  };
  useEffect(() => {
    const destination = takeRequestedSettingField();
    if (destination) navigateToSetting(destination);
  }, []);
  const nativeDirty = {
    keys: false, management: Boolean(managementSecretDraft || managementSecretConfirm),
    network: networkSettingsDirty, tls: tlsSettingsDirty, routing: sessionRoutingDirty,
    retry: retrySettingsDirty, logging: loggingSettingsDirty, software: softwareSettingsDirty,
  };
  const nativeEntries: SettingSearchEntry[] = [
    { category: 'general', target: 'config-native-keys', title: t('config.keys.title'), context: '', keywords: 'API key token 鉴权密钥 api-keys access' },
    { category: 'general', target: 'config-native-management', title: t('config.webuiKey.title'), context: '', keywords: 'WebUI secret-key 密钥 管理接口 面板 密码 management security' },
    { category: 'general', target: 'config-native-network', title: st('nativeNetwork'), context: '', keywords: [t('config.network.port'), t('config.network.listenHost'), t('config.network.proxyUrl'), t('config.network.systemProxy'), 'server host port proxy URL 8317 127.0.0.1 0.0.0.0 监听 地址 系统代理'].join(' ') },
    { category: 'general', target: 'config-native-tls', title: t('config.tls.enable'), context: '', keywords: [t('config.tls.cert'), t('config.tls.key'), 'TLS HTTPS cert key certificate 证书 私钥 加密'].join(' ') },
    { category: 'routing', target: 'config-native-routing', title: st('nativeRouting'), context: '', keywords: [t('config.network.sessionAffinity'), t('config.network.sessionTtl'), t('config.routing.title'), 'session affinity ttl routing strategy round-robin 会话 粘性 加权 轮询'].join(' ') },
    { category: 'requests', target: 'config-native-retry', title: st('nativeRetry'), context: '', keywords: [t('config.network.enableCooling'), t('config.network.disableCooling'), t('config.network.requestRetry'), t('config.network.maxRetryCredentials'), t('config.network.maxRetryInterval'), t('config.network.streamingBootstrapRetries'), 'retry retries cooldown 重试 冷却 流式 失败 fallback'].join(' ') },
    { category: 'diagnostics', target: 'config-native-logging', title: t('config.diagnostics.title'), context: '', keywords: [t('config.diagnostics.debug.title'), t('config.diagnostics.commercial.title'), t('config.diagnostics.fileLogging.title'), t('config.diagnostics.usage.title'), t('config.diagnostics.maxSize.title'), t('config.diagnostics.errorFiles.title'), t('config.diagnostics.redisRetention.title'), 'debug commercial-mode logging logs usage statistics redis retention 日志 调试 商业模式 用量 统计 留存 文件 容量'].join(' ') },
    { category: 'software', target: 'config-native-software', title: t('config.software.title'), context: '', keywords: [t('config.software.autostart'), t('config.software.startCoreOnLaunch'), t('config.software.silentStart'), t('config.software.closeBehavior'), t('config.software.defaultTerminal'), 'startup autostart silent tray close terminal 软件 自启动 启动 静默 托盘 关闭 终端'].join(' ') },
    { category: 'aliases', target: 'config-native-aliases', title: t('app.nav.thinkingAliases'), context: '', keywords: 'thinking reasoning speed aliases effort 思考 推理 速度 别名 模型' },
    { category: 'requests', target: 'config-native-sensitive-words', title: t('config.sensitiveWords.title'), context: '', keywords: 'sensitive words filters antigravity devin 敏感词 内容 过滤' },
  ];
  const nativeCategoryDirty: Record<ConfigSubpage, boolean> = {
    general: nativeDirty.management || networkSettingsDirty || tlsSettingsDirty,
    aliases: false,
    routing: sessionRoutingDirty,
    requests: sensitiveWordsDirty || retrySettingsDirty,
    diagnostics: loggingSettingsDirty,
    oauth: false,
    extensions: false,
    software: softwareSettingsDirty,
  };
  const categoryDirty = (id: ConfigSubpage) => nativeCategoryDirty[id]
    || settingsTemplateGroups[id].some(group => dirtyTemplateGroups.includes(group.id));
  const templateGroups = settingsTemplateGroups[activeSubpage];
  const searchEntries: SettingSearchEntry[] = [
    ...nativeEntries,
    ...settingsCategories.flatMap(item => settingsTemplateGroups[item.id].flatMap(group => group.fields.map((field, index) => ({
      category: item.id,
      target: `config-group-${group.id}`, field: `template-field-${group.id}-${index}`,
      title: templateText(field.label, locale), context: templateText(group.title, locale),
      keywords: `${JSON.stringify(field)} ${templateText(group.description, locale)} ${field.path.join('.')}`,
    })))),
  ];
  const searchTerms = settingsSearch.trim().toLocaleLowerCase().split(/\s+/);
  const searchResults = searchEntries.filter(entry => {
    const categoryTitle = templateText(settingsCategories.find(item => item.id === entry.category)!.title, locale);
    const text = `${entry.title} ${entry.context} ${entry.keywords} ${categoryTitle}`.toLocaleLowerCase();
    return searchTerms.every(term => text.includes(term));
  });

  return (
    <section className="page config-page">
      {confirmationDialog}
      <header className="config-settings-header">
        <h1>{st('title')}</h1>
        <div className="config-settings-search"><Search size={18} aria-hidden="true" /><input ref={searchRef} type="search" aria-label={st('search')} placeholder={st('searchHint')} value={settingsSearch} onChange={event => setSettingsSearch(event.currentTarget.value)} onKeyDown={event => { if (event.key === 'Escape') setSettingsSearch(''); }} />{searching ? <button type="button" className="icon-button quiet" aria-label={st('clearSearch')} onClick={() => { setSettingsSearch(''); searchRef.current?.focus(); }}><X size={16} /></button> : null}</div>
      </header>
      <div className="config-settings-layout">
        <div className="config-settings-topnav">
          <div ref={categoryNavigationRef} className="config-subpage-tabs config-settings-navigation" role="tablist" aria-orientation="horizontal" aria-label={t('config.tabs.label')}>
            {settingsCategories.map(item => {
              const Icon = CATEGORY_ICONS[item.id];
              const dirty = categoryDirty(item.id);
              return <button type="button" key={item.id} id={`config-subpage-tab-${item.id}`} role="tab" className={activeSubpage === item.id ? 'active' : ''} aria-selected={activeSubpage === item.id} aria-controls="config-subpage-panel" tabIndex={activeSubpage === item.id ? 0 : -1} onClick={() => activateConfigSubpage(item.id)} onKeyDown={event => handleConfigTabKeyDown(event, item.id)}><Icon size={17} aria-hidden="true" /><span>{templateText(item.title, locale)}</span>{dirty ? <span className="config-nav-dirty" title={st('dirty')}><span className="sr-only">{st('dirty')}</span></span> : null}</button>;
            })}
          </div>
        </div>
        <div className="config-settings-content" id="config-subpage-panel" role="tabpanel" aria-labelledby={`config-subpage-tab-${activeSubpage}`}>
          {searching ? <section className="config-search-results" aria-label={st('results')}><div className="config-search-heading"><h2>{st('results')}</h2><span role="status">{searchResults.length}</span></div>{searchResults.length ? <div className="config-search-result-list">{searchResults.map(entry => <button type="button" key={`${entry.target}-${entry.field ?? ''}`} onClick={() => navigateToSetting(entry)}><span className="config-search-result-copy"><strong>{entry.title}</strong><small>{templateText(settingsCategories.find(item => item.id === entry.category)!.title, locale)}{entry.context ? ` / ${entry.context}` : ''}</small></span><ChevronRight size={17} aria-hidden="true" /></button>)}</div> : <p className="config-search-empty">{st('noResults')}</p>}</section> : null}
          <div className="config-settings-cards">
<section id="config-native-keys" tabIndex={-1} hidden={searching || activeSubpage !== 'general'} className="panel config-keys-panel">
          <div className="config-panel-heading">
            <div className="config-heading-title">
              <KeyRound size={18} aria-hidden="true" />
              <h2>{t('config.keys.title')}</h2>
            </div>
            <div className="config-heading-actions">
              <span className="config-count" aria-label={t('config.keys.count')}>
                {settings?.apiKeys.length ?? 0}
              </span>
              <button
                type="button"
                className="icon-button"
                onClick={openAddDialog}
                disabled={controlsDisabled}
                title={t('config.keys.add')}
                aria-label={t('config.keys.add')}
              >
                <Plus size={18} aria-hidden="true" />
              </button>
            </div>
          </div>

          <div className="config-key-list" aria-busy={loading || undefined}>
            {loading ? (
              Array.from({ length: 5 }, (_, index) => (
                <div className="config-key-row skeleton" key={index} aria-hidden="true">
                  <span />
                  <span />
                </div>
              ))
            ) : loadError ? (
              <div className="config-unavailable">
                <AlertCircle size={24} aria-hidden="true" />
                <strong>{t('config.unavailable')}</strong>
                <span title={loadError}>{loadError}</span>
                <button type="button" className="secondary-button compact-button" onClick={() => void loadSettings()}>
                  <RefreshCw size={16} aria-hidden="true" />
                  {t('common.retry')}
                </button>
              </div>
            ) : settings && settings.apiKeys.length > 0 ? (
              settings.apiKeys.map((entry, index) => (
                <div className="config-key-row" key={`${index}-${entry.apiKey}`}>
                  <div className="config-key-identity">
                    <span className="config-key-index">{String(index + 1).padStart(2, '0')}</span>
                    <div className="config-key-details">
                      <div className="config-key-label-line">
                        <strong title={entry.remark || t('config.keys.noRemark')}>
                          {entry.remark || t('config.keys.noRemark')}
                        </strong>
                      </div>
                      <code title={maskApiKey(entry.apiKey)}>{maskApiKey(entry.apiKey)}</code>
                    </div>
                  </div>
                  <div className="config-key-actions">
                    <button
                      type="button"
                      className="icon-button quiet"
                      onClick={() => void copyApiKey(entry.apiKey, index)}
                      disabled={controlsDisabled}
                      title={copiedIndex === index ? t('config.notice.keyCopied') : t('config.keys.copy')}
                      aria-label={copiedIndex === index ? t('config.notice.keyCopied') : t('config.keys.copyNth', { number: index + 1 })}
                    >
                      {copiedIndex === index ? (
                        <Check size={16} aria-hidden="true" />
                      ) : (
                        <Copy size={16} aria-hidden="true" />
                      )}
                    </button>
                    <button
                      type="button"
                      className="icon-button quiet"
                      onClick={() => openEditDialog(entry)}
                      disabled={controlsDisabled}
                      title={t('config.keys.edit')}
                      aria-label={t('config.keys.editNth', { number: index + 1 })}
                    >
                      <Pencil size={16} aria-hidden="true" />
                    </button>
                    <button
                      type="button"
                      className="icon-button danger"
                      onClick={() => setDeleteIndex(index)}
                      disabled={controlsDisabled}
                      title={t('config.keys.delete')}
                      aria-label={t('config.keys.deleteNth', { number: index + 1 })}
                    >
                      <Trash2 size={16} aria-hidden="true" />
                    </button>
                  </div>
                </div>
              ))
            ) : (
              <div className="config-empty-list">
                <KeyRound size={26} aria-hidden="true" />
                <strong>{t('config.keys.empty')}</strong>
              </div>
            )}
          </div>
          {!addDialogOpen && deleteIndex === null ? renderFeedback(keyFeedback) : null}
        </section>
<section id="config-native-management" tabIndex={-1} hidden={searching || activeSubpage !== 'general'} className="panel config-management-panel">
          <div className="config-panel-heading">
            <div className="config-heading-title">
              <ShieldCheck size={18} aria-hidden="true" />
              <h2>{t('config.webuiKey.title')}</h2><SettingsHelp label={t('config.webuiKey.title')}>{t('config.webuiKey.description')} {t('config.webuiKey.securityHint')}</SettingsHelp>
            </div>
            <div className="config-heading-actions">
              <button
                type="button"
                className="secondary-button compact-button"
                disabled={loading || settings === null}
                onClick={() => void openWebUi()}
                title={settings ? webUiManagementUrl(settings.port, tlsSettings?.enabled, settings.host) : undefined}
              >
                {t('config.webuiKey.open')}
              </button>
            </div>
          </div>

          <div className="config-management-content">
            {settings?.managementSecretConfigured === false ? <p role="status">{templateText(templateMessages.managementDisabled, locale)}</p> : null}

            <form
              className="config-management-form"
              onSubmit={(event) => void saveManagementSecret(event)}
            >
              <label className="config-management-field">
                <span>{t('config.webuiKey.newKey')}</span>
                <div className="config-secret-input">
                  <input
                    type={showManagementSecret ? 'text' : 'password'}
                    autoComplete="new-password"
                    maxLength={512}
                    value={managementSecretDraft}
                    disabled={controlsDisabled}
                    aria-invalid={Boolean(managementSecretError)}
                    placeholder={t('config.webuiKey.placeholder')}
                    onChange={(event) => {
                      setManagementSecretDraft(event.currentTarget.value);
                      setManagementSecretError('');
                    }}
                  />
                  <button
                    type="button"
                    className="icon-button quiet"
                    disabled={controlsDisabled}
                    onClick={() => setShowManagementSecret((value) => !value)}
                    title={showManagementSecret ? t('config.keys.hide') : t('config.keys.show')}
                    aria-label={showManagementSecret ? t('config.keys.hide') : t('config.keys.show')}
                  >
                    {showManagementSecret ? (
                      <EyeOff size={16} aria-hidden="true" />
                    ) : (
                      <Eye size={16} aria-hidden="true" />
                    )}
                  </button>
                </div>
              </label>

              <label className="config-management-field">
                <span>{t('config.webuiKey.confirmKey')}</span>
                <input
                  className="config-dialog-text-input"
                  type={showManagementSecret ? 'text' : 'password'}
                  autoComplete="new-password"
                  maxLength={512}
                  value={managementSecretConfirm}
                  disabled={controlsDisabled}
                  aria-invalid={Boolean(managementSecretError)}
                  placeholder={t('config.webuiKey.confirmPlaceholder')}
                  onChange={(event) => {
                    setManagementSecretConfirm(event.currentTarget.value);
                    setManagementSecretError('');
                  }}
                />
              </label>

              <div className="config-management-form-footer">
                <span
                  className={`config-management-error ${managementSecretError ? 'visible' : ''}`}
                  role="alert"
                >
                  {managementSecretError || ' '}
                </span>
                <div className="config-management-actions">
                  <button type="button" className="secondary-button compact-button" disabled={controlsDisabled || settings?.managementSecretConfigured === false} onClick={() => void disableManagement()}>
                    {templateText(templateMessages.disableManagement, locale)}
                  </button>
                  <button
                    type="button"
                    className="secondary-button compact-button"
                  disabled={controlsDisabled}
                  onClick={generateManagementSecret}
                >
                    {t('config.webuiKey.generate')}
                  </button>
                  <button
                    type="submit"
                    className="primary-button compact-button"
                    disabled={controlsDisabled || !managementSecretDraft.trim()}
                  >
                    <Check size={16} aria-hidden="true" />
                    {managementSecretBusy ? t('common.saving') : t('config.webuiKey.save')}
                  </button>
                </div>
              </div>
            </form>
            {renderFeedback(managementFeedback)}
          </div>
        </section>
<section id="config-native-network" tabIndex={-1} hidden={searching || activeSubpage !== 'general'}
            className="config-network-section"
            aria-labelledby="config-network-section-title"
          >
            <div className="config-network-section-heading config-section-heading-with-actions">
              <div className="config-network-section-title">
                <Network size={16} aria-hidden="true" />
                <h3 id="config-network-section-title">{st('nativeNetwork')}</h3>
              </div>
              <span className="config-card-status" role="status">{nativeDirty.network ? st('dirty') : ''}</span>

            </div>
            {renderFeedback(networkFeedback)}
            <div className="config-network-grid">
              <div className="config-network-field config-network-port-field">
            <span className="config-field-label"><label htmlFor="config-input-config-network-port">{t('config.network.port')}</label><SettingsHelp label={t('config.network.port')}>{t('config.network.portHint')}</SettingsHelp></span>
            <input
                  id="config-input-config-network-port"
              className={`config-network-input ${portError ? 'error' : ''}`}
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              maxLength={5}
              value={portDraft}
              disabled={controlsDisabled}
              aria-invalid={Boolean(portError)}
              title={portError || t('config.network.portHint')}
              onChange={(event) => {
                markDraftDirty('port');
                setPortDraft(event.currentTarget.value.replace(/\D/g, '').slice(0, 5));
                setPortError('');
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape' && settings) {
                  clearDraftDirty('port');
                  setPortDraft(String(settings.port));
                  setPortError('');
                  event.currentTarget.blur();
                }
              }}
            />

              </div>

              <div className="config-network-field">
                <span className="config-field-label"><label htmlFor="config-input-config-network-listenHost">{t('config.network.listenHost')}</label><SettingsHelp label={t('config.network.listenHost')}>{templateText(templateMessages.hostHint, locale)}</SettingsHelp></span>
                <input
                  id="config-input-config-network-listenHost"
                  className={`config-network-input ${hostError ? 'error' : ''}`}
                  type="text"
                  value={hostDraft}
                  disabled={controlsDisabled}
                  placeholder="127.0.0.1"
                  aria-invalid={Boolean(hostError)}
                  title={hostError || templateText(templateMessages.hostHint, locale)}
                  onChange={(event) => {
                    markDraftDirty('host');
                    setHostDraft(event.currentTarget.value);
                    setHostError('');
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('host');
                      setHostDraft(settings.host);
                      setHostError('');
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>

              <div className="config-network-field config-proxy-field">
                <div className="config-proxy-heading">
                  <label className="config-network-label" htmlFor="config-network-proxy-url">
                    <Link2 size={15} aria-hidden="true" />
                    {t('config.network.proxyUrl')}
                  </label>
                  <SettingsHelp label={t('config.network.proxyUrl')}>{t('config.network.proxyHint')} {t('config.network.systemProxyHint')}</SettingsHelp>
                  <label className="config-proxy-system-toggle" title={t('config.network.systemProxyHint')}>
                    <span>{t('config.network.systemProxy')}</span>
                    <span className="switch-control">
                      <input
                        type="checkbox"
                        checked={!proxyOverrideDraft}
                        disabled={controlsDisabled}
                        onChange={(event) => {
                          markDraftDirty('proxyOverride');
                          setProxyOverrideDraft(!event.currentTarget.checked);
                        }}
                      />
                      <span className="switch-track" />
                    </span>
                  </label>
                </div>
                <input
                  id="config-network-proxy-url"
                  className="config-network-input"
                  type="text"
                  value={proxyUrlDraft}
                  disabled={controlsDisabled || !proxyOverrideDraft}
                  placeholder={t('config.network.proxyPlaceholder')}
                  onChange={(event) => {
                    markDraftDirty('proxyUrl');
                    setProxyUrlDraft(event.currentTarget.value);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('proxyUrl');
                      setProxyUrlDraft(settings.proxyUrl);
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>
            </div>

          <div className="config-settings-save-row"><button
                type="button"
                className="primary-button compact-button"
                disabled={controlsDisabled || !networkSettingsDirty}
                onClick={() => void saveNetworkEndpointSettings()}
              >
                <Check size={16} aria-hidden="true" />
                {busyAction === 'network' ? t('common.saving') : t('common.save')}
              </button></div>
        </section>
<section id="config-native-tls" tabIndex={-1} hidden={searching || activeSubpage !== 'general'}
            className="config-network-section"
            aria-labelledby="config-tls-section-title"
          >
          <div className="config-network-section-heading config-tls-section-heading">
            <div className="config-tls-section-title">
              <LockKeyhole size={18} aria-hidden="true" />
              <h3 id="config-tls-section-title">{t('config.tls.enable')}</h3>
            </div>
            <div className="config-heading-actions">
              {tlsStatusLabel ? (
                <span className={`state-pill ${tlsStatusIsSaved ? 'success' : ''}`}>
                  {tlsStatusLabel}
                </span>
              ) : null}

            </div>
          </div>

          <div className="config-tls-content">
            <div className="config-software-setting-row config-tls-toggle-row">
              <div className="config-software-setting-copy">
                <span className="config-software-setting-icon" aria-hidden="true">
                  <ShieldCheck size={18} />
                </span>
                <div>
                  <span className="config-field-label"><strong>{t('config.tls.enable')}</strong><SettingsHelp label={t('config.tls.enable')}>{t('config.tls.enableDescription')} {t('config.tls.restartHint')}</SettingsHelp></span>

                </div>
              </div>
              <label className="switch-control" title={t('config.tls.enable')}>
                <input
                  type="checkbox"
                  role="switch"
                  aria-label={t('config.tls.enable')}
                  checked={tlsEnabledDraft}
                  disabled={tlsSettingsLoading || tlsSettings === null || busyAction !== null}
                  onChange={(event) => {
                    setTlsSavedStatusVisible(false);
                    setTlsError('');
                    setTlsEnabledDraft(event.currentTarget.checked);
                  }}
                />
                <span className="switch-track" />
              </label>
            </div>

            {tlsEnabledDraft ? (
              <div className="config-tls-fields">
                <div className="config-network-field">
                  <span className="config-field-label"><span>{t('config.tls.cert')}</span><SettingsHelp label={t('config.tls.cert')}>{t('config.tls.certHint')}</SettingsHelp></span>
                  <div className="config-tls-path-control">
                    <input
                      className={`config-network-input ${tlsError && !tlsCertDraft.trim() ? 'error' : ''}`}
                      type="text"
                      value={tlsCertDraft}
                      aria-label={t('config.tls.cert')}
                      disabled={tlsSettingsLoading || tlsSettings === null || busyAction !== null || tlsFileSelecting !== null}
                      placeholder={t('config.tls.certPlaceholder')}
                      onChange={(event) => {
                        setTlsSavedStatusVisible(false);
                        setTlsError('');
                        setTlsCertDraft(event.currentTarget.value);
                      }}
                    />
                    <button
                      type="button"
                      className="secondary-button compact-button config-tls-browse-button"
                      disabled={tlsSettingsLoading || tlsSettings === null || busyAction !== null || tlsFileSelecting !== null}
                      title={t('config.tls.selectCertTitle')}
                      onClick={() => void selectTlsFile('cert')}
                    >
                      <FolderOpen size={16} aria-hidden="true" />
                      {tlsFileSelecting === 'cert' ? t('common.processing') : t('config.tls.browse')}
                    </button>
                  </div>

                </div>
                <div className="config-network-field">
                  <span className="config-field-label"><span>{t('config.tls.key')}</span><SettingsHelp label={t('config.tls.key')}>{t('config.tls.keyHint')}</SettingsHelp></span>
                  <div className="config-tls-path-control">
                    <input
                      className={`config-network-input ${tlsError && !tlsKeyDraft.trim() ? 'error' : ''}`}
                      type="text"
                      value={tlsKeyDraft}
                      aria-label={t('config.tls.key')}
                      disabled={tlsSettingsLoading || tlsSettings === null || busyAction !== null || tlsFileSelecting !== null}
                      placeholder={t('config.tls.keyPlaceholder')}
                      onChange={(event) => {
                        setTlsSavedStatusVisible(false);
                        setTlsError('');
                        setTlsKeyDraft(event.currentTarget.value);
                      }}
                    />
                    <button
                      type="button"
                      className="secondary-button compact-button config-tls-browse-button"
                      disabled={tlsSettingsLoading || tlsSettings === null || busyAction !== null || tlsFileSelecting !== null}
                      title={t('config.tls.selectKeyTitle')}
                      onClick={() => void selectTlsFile('key')}
                    >
                      <FolderOpen size={16} aria-hidden="true" />
                      {tlsFileSelecting === 'key' ? t('common.processing') : t('config.tls.browse')}
                    </button>
                  </div>

                </div>
              </div>
            ) : null}

            <MessageNotice message={tlsError} onDismiss={() => setTlsError('')} />

            {renderFeedback(tlsFeedback)}
          </div>

          <div className="config-settings-save-row"><button
                type="button"
                className="primary-button compact-button"
                disabled={tlsSettingsLoading || tlsSettings === null || busyAction !== null || tlsFileSelecting !== null || !tlsSettingsDirty}
                onClick={() => void saveTlsSettings()}
              >
                <Check size={16} aria-hidden="true" />
                {busyAction === 'tls' ? t('common.saving') : t('common.save')}
              </button></div>
        </section>
<section id="config-native-routing" tabIndex={-1} hidden={searching || activeSubpage !== 'routing'}
            className="config-network-section"
            aria-labelledby="config-routing-section-title"
          >
            <div className="config-network-section-heading config-section-heading-with-actions">
              <div className="config-network-section-title">
                <Route size={16} aria-hidden="true" />
                <h3 id="config-routing-section-title">{st('nativeRouting')}</h3>
              </div>
              <span className="config-card-status" role="status">{nativeDirty.routing ? st('dirty') : ''}</span>

            </div>
            {renderFeedback(routingFeedback)}
            <div className="config-network-grid">
              <div className="config-network-field config-network-toggle">
                <div>
                  <span className="config-field-label"><span>{t('config.network.sessionAffinity')}</span><SettingsHelp label={t('config.network.sessionAffinity')}>{t('config.network.sessionAffinityHint')}</SettingsHelp></span>

                </div>
                <label className="switch-control" title={t('config.network.sessionAffinity')}>
                  <input
                    type="checkbox"
                    aria-label={t('config.network.sessionAffinity')}
                    checked={sessionAffinityDraft}
                    disabled={controlsDisabled}
                    onChange={(event) => {
                      markDraftDirty('sessionAffinity');
                      setSessionAffinityDraft(event.currentTarget.checked);
                    }}
                  />
                  <span className="switch-track" />
                </label>
              </div>

              <div className="config-network-field">
                <span className="config-field-label"><label htmlFor="config-input-config-network-sessionTtl">
                  <Clock3 size={15} aria-hidden="true" />
                  {t('config.network.sessionTtl')}
                </label><SettingsHelp label={t('config.network.sessionTtl')}>{t('config.network.sessionTtlHint')}</SettingsHelp></span>
                <input
                  id="config-input-config-network-sessionTtl"
                  className="config-network-input"
                  type="text"
                  value={sessionTtlDraft}
                  disabled={controlsDisabled}
                  placeholder="1h"
                  onChange={(event) => {
                    markDraftDirty('sessionTtl');
                    setSessionTtlDraft(event.currentTarget.value);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('sessionTtl');
                      setSessionTtlDraft(settings.routingSessionAffinityTtl);
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>

              <div className="config-network-field config-network-routing-field">
                <span className="config-network-label">
                  <Route size={15} aria-hidden="true" />
                  {t('config.routing.title')}
                  <span className="config-inline-status">{st('automatic')}</span>
                </span>
                <div className="routing-segmented" role="group" aria-label={t('config.routing.title')}>
                  {ROUTING_OPTIONS.map((option) => (
                    <button
                      type="button"
                      key={option.value}
                      className={settings?.routingStrategy === option.value ? 'active' : ''}
                      aria-pressed={settings?.routingStrategy === option.value}
                      disabled={controlsDisabled}
                      onClick={() => void changeRoutingStrategy(option.value)}
                      title={option.value}
                    >
                      {option.value === 'weighted-round-robin' ? templateText(templateMessages.weighted, locale) : t(option.labelKey)}
                    </button>
                  ))}
                </div>
                <small className="sr-only" title={settings?.routingStrategy || undefined}>
                  {loading
                    ? t('common.loading')
                    : settings === null
                      ? t('common.unavailable')
                      : routingStrategyLabel(settings.routingStrategy, t, locale)}
                </small>
              </div>
            </div>

          <div className="config-settings-save-row"><button
                type="button"
                className="primary-button compact-button"
                disabled={controlsDisabled || !sessionRoutingDirty}
                onClick={() => void saveSessionRoutingSettings()}
              >
                <Check size={16} aria-hidden="true" />
                {busyAction === 'routing' ? t('common.saving') : t('common.save')}
              </button></div>
        </section>
<section id="config-native-retry" tabIndex={-1} hidden={searching || activeSubpage !== 'requests'}
            className="config-network-section"
            aria-labelledby="config-retry-section-title"
          >
            <div className="config-network-section-heading config-section-heading-with-actions">
              <div className="config-network-section-title">
                <RefreshCw size={16} aria-hidden="true" />
                <h3 id="config-retry-section-title">{st('nativeRetry')}</h3>
              </div>
              <span className="config-card-status" role="status">{nativeDirty.retry ? st('dirty') : ''}</span>

            </div>
            {renderFeedback(retryFeedback)}
            <div className="config-network-grid">
              <CoolingToggle disabledCooling={disableCoolingDraft} disabled={controlsDisabled}
                onChange={(value) => {
                  markDraftDirty('disableCooling');
                  setDisableCoolingDraft(value);
                }} />

              <div className="config-network-field">
                <span className="config-field-label"><label htmlFor="config-input-config-network-requestRetry">{t('config.network.requestRetry')}</label><SettingsHelp label={t('config.network.requestRetry')}>{templateText(templateMessages.retryHint, locale)}</SettingsHelp></span>
                <input
                  id="config-input-config-network-requestRetry"
                  className={`config-network-input ${retryError ? 'error' : ''}`}
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  maxLength={10}
                  value={requestRetryDraft}
                  disabled={controlsDisabled}
                  aria-invalid={Boolean(retryError)}
                  onChange={(event) => {
                    markDraftDirty('requestRetry');
                    setRequestRetryDraft(event.currentTarget.value.replace(/\D/g, '').slice(0, 10));
                    setRetryError('');
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('requestRetry');
                      setRequestRetryDraft(String(settings.requestRetry));
                      setRetryError('');
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>

              <div className="config-network-field">
                <span className="config-field-label"><label htmlFor="config-input-config-network-maxRetryCredentials">{t('config.network.maxRetryCredentials')}</label><SettingsHelp label={t('config.network.maxRetryCredentials')}>{templateText(templateMessages.credentialsHint, locale)}</SettingsHelp></span>
                <input
                  id="config-input-config-network-maxRetryCredentials"
                  className={`config-network-input ${retryError ? 'error' : ''}`}
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  maxLength={10}
                  value={maxRetryCredentialsDraft}
                  disabled={controlsDisabled}
                  aria-invalid={Boolean(retryError)}
                  onChange={(event) => {
                    markDraftDirty('maxRetryCredentials');
                    setMaxRetryCredentialsDraft(event.currentTarget.value.replace(/\D/g, '').slice(0, 10));
                    setRetryError('');
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('maxRetryCredentials');
                      setMaxRetryCredentialsDraft(String(settings.maxRetryCredentials));
                      setRetryError('');
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>

              <div className="config-network-field">
                <span className="config-field-label"><label htmlFor="config-input-config-network-maxRetryInterval">{t('config.network.maxRetryInterval')}</label><SettingsHelp label={t('config.network.maxRetryInterval')}>{templateText(templateMessages.waitHint, locale)}</SettingsHelp></span>
                <input
                  id="config-input-config-network-maxRetryInterval"
                  className={`config-network-input ${retryError ? 'error' : ''}`}
                  type="text"
                  inputMode="numeric"
                  pattern="-?[0-9]*"
                  maxLength={11}
                  value={maxRetryIntervalDraft}
                  disabled={controlsDisabled}
                  aria-invalid={Boolean(retryError)}
                  onChange={(event) => {
                    markDraftDirty('maxRetryInterval');
                    const raw = event.currentTarget.value;
                    setMaxRetryIntervalDraft(`${raw.startsWith('-') ? '-' : ''}${raw.replace(/\D/g, '').slice(0, 10)}`);
                    setRetryError('');
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('maxRetryInterval');
                      setMaxRetryIntervalDraft(String(settings.maxRetryInterval));
                      setRetryError('');
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>

              <div className="config-network-field">
                <span className="config-field-label"><label htmlFor="config-input-config-network-streamingBootstrapRetries">{t('config.network.streamingBootstrapRetries')}</label><SettingsHelp label={t('config.network.streamingBootstrapRetries')}>{t('config.network.streamingBootstrapRetriesHint')}</SettingsHelp></span>
                <input
                  id="config-input-config-network-streamingBootstrapRetries"
                  className={`config-network-input ${retryError ? 'error' : ''}`}
                  type="text"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  maxLength={10}
                  value={streamingBootstrapRetriesDraft}
                  disabled={controlsDisabled}
                  aria-invalid={Boolean(retryError)}
                  onChange={(event) => {
                    markDraftDirty('streamingBootstrapRetries');
                    setStreamingBootstrapRetriesDraft(event.currentTarget.value.replace(/\D/g, '').slice(0, 10));
                    setRetryError('');
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Escape' && settings) {
                      clearDraftDirty('streamingBootstrapRetries');
                      setStreamingBootstrapRetriesDraft(String(settings.streamingBootstrapRetries));
                      setRetryError('');
                      event.currentTarget.blur();
                    }
                  }}
                />

              </div>
            </div>

          <div className="config-settings-save-row"><button
                type="button"
                className="primary-button compact-button"
                disabled={controlsDisabled || !retrySettingsDirty}
                onClick={() => void saveRetrySettings()}
              >
                <Check size={16} aria-hidden="true" />
                {busyAction === 'retry' ? t('common.saving') : t('common.save')}
              </button></div>
        </section>
<section id="config-native-logging" tabIndex={-1} hidden={searching || activeSubpage !== 'diagnostics'} className="panel config-diagnostics-panel">
          <div className="config-panel-heading">
            <div className="config-heading-title">
              <FileText size={18} aria-hidden="true" />
              <h2>{t('config.diagnostics.title')}</h2>
            </div>
          </div>

          <div className="config-diagnostics-toggle-grid">
            <div className="config-diagnostics-setting">
              <span className="config-diagnostics-setting-icon" aria-hidden="true"><Bug size={18} /></span>
              <div className="config-diagnostics-setting-copy">
                <span className="config-field-label"><strong>{t('config.diagnostics.debug.title')}</strong><SettingsHelp label={t('config.diagnostics.debug.title')}>{t('config.diagnostics.debug.description')}</SettingsHelp></span>

              </div>
              <label className="switch-control" title={t('config.diagnostics.debug.title')}>
                <input
                  type="checkbox"
                  role="switch"
                  checked={debugDraft}
                  aria-label={t('config.diagnostics.debug.title')}
                  disabled={controlsDisabled}
                  onChange={(event) => {
                    setDebugDraft(event.currentTarget.checked);
                    markLoggingDraftDirty();
                  }}
                />
                <span className="switch-track" />
              </label>
            </div>

            <div className="config-diagnostics-setting">
              <span className="config-diagnostics-setting-icon" aria-hidden="true"><Gauge size={18} /></span>
              <div className="config-diagnostics-setting-copy">
                <span className="config-field-label"><strong>{t('config.diagnostics.commercial.title')}</strong><SettingsHelp label={t('config.diagnostics.commercial.title')}>{t('config.diagnostics.commercial.description')} {t('config.diagnostics.commercial.warning')}</SettingsHelp></span>

              </div>
              <label className="switch-control" title={t('config.diagnostics.commercial.title')}>
                <input
                  type="checkbox"
                  role="switch"
                  checked={commercialModeDraft}
                  aria-label={t('config.diagnostics.commercial.title')}
                  disabled={controlsDisabled}
                  onChange={(event) => {
                    setCommercialModeDraft(event.currentTarget.checked);
                    markLoggingDraftDirty();
                  }}
                />
                <span className="switch-track" />
              </label>
            </div>

            <div className="config-diagnostics-setting">
              <span className="config-diagnostics-setting-icon" aria-hidden="true"><HardDrive size={18} /></span>
              <div className="config-diagnostics-setting-copy">
                <span className="config-field-label"><strong>{t('config.diagnostics.fileLogging.title')}</strong><SettingsHelp label={t('config.diagnostics.fileLogging.title')}>{t('config.diagnostics.fileLogging.description')}</SettingsHelp></span>

              </div>
              <label className="switch-control" title={t('config.diagnostics.fileLogging.title')}>
                <input
                  type="checkbox"
                  role="switch"
                  checked={loggingToFileDraft}
                  aria-label={t('config.diagnostics.fileLogging.title')}
                  disabled={controlsDisabled}
                  onChange={(event) => {
                    setLoggingToFileDraft(event.currentTarget.checked);
                    markLoggingDraftDirty();
                  }}
                />
                <span className="switch-track" />
              </label>
            </div>

            <div className="config-diagnostics-setting">
              <span className="config-diagnostics-setting-icon" aria-hidden="true"><Database size={18} /></span>
              <div className="config-diagnostics-setting-copy">
                <span className="config-field-label"><strong>{t('config.diagnostics.usage.title')}</strong><SettingsHelp label={t('config.diagnostics.usage.title')}>{t('config.diagnostics.usage.description')}</SettingsHelp></span>

              </div>
              <label className="switch-control" title={t('config.diagnostics.usage.title')}>
                <input
                  type="checkbox"
                  role="switch"
                  checked={usageStatisticsDraft}
                  aria-label={t('config.diagnostics.usage.title')}
                  disabled={controlsDisabled}
                  onChange={(event) => {
                    setUsageStatisticsDraft(event.currentTarget.checked);
                    markLoggingDraftDirty();
                  }}
                />
                <span className="switch-track" />
              </label>
            </div>
          </div>

          <div className="config-diagnostics-fields">
            <div className="config-diagnostics-field">
              <span className="config-field-label"><label htmlFor="config-input-config-diagnostics-maxSize-title">{t('config.diagnostics.maxSize.title')}</label><SettingsHelp label={t('config.diagnostics.maxSize.title')}>{t('config.diagnostics.maxSize.hint')}</SettingsHelp></span>
              <input
                  id="config-input-config-diagnostics-maxSize-title"
                className="config-dialog-text-input"
                type="number"
                min="0"
                step="1"
                value={logsMaxTotalSizeDraft}
                disabled={controlsDisabled}
                onChange={(event) => {
                  setLogsMaxTotalSizeDraft(event.currentTarget.value);
                  markLoggingDraftDirty();
                }}
              />

            </div>
            <div className="config-diagnostics-field">
              <span className="config-field-label"><label htmlFor="config-input-config-diagnostics-errorFiles-title">{t('config.diagnostics.errorFiles.title')}</label><SettingsHelp label={t('config.diagnostics.errorFiles.title')}>{t('config.diagnostics.errorFiles.hint')}</SettingsHelp></span>
              <input
                  id="config-input-config-diagnostics-errorFiles-title"
                className="config-dialog-text-input"
                type="number"
                min="0"
                step="1"
                value={errorLogsMaxFilesDraft}
                disabled={controlsDisabled}
                onChange={(event) => {
                  setErrorLogsMaxFilesDraft(event.currentTarget.value);
                  markLoggingDraftDirty();
                }}
              />

            </div>
            <div className="config-diagnostics-field">
              <span className="config-field-label"><label htmlFor="config-input-config-diagnostics-redisRetention-title">{t('config.diagnostics.redisRetention.title')}</label><SettingsHelp label={t('config.diagnostics.redisRetention.title')}>{t('config.diagnostics.redisRetention.hint')}</SettingsHelp></span>
              <input
                  id="config-input-config-diagnostics-redisRetention-title"
                className="config-dialog-text-input"
                type="number"
                min="1"
                max="3600"
                step="1"
                value={redisUsageRetentionDraft}
                disabled={controlsDisabled}
                onChange={(event) => {
                  setRedisUsageRetentionDraft(event.currentTarget.value);
                  markLoggingDraftDirty();
                }}
              />

            </div>
          </div>

          {settings && commercialModeDraft !== settings.commercialMode ? (
            <div className="config-diagnostics-commercial-note">
              <AlertCircle size={16} aria-hidden="true" />
              <span>{st('restart')}</span>
            </div>
          ) : null}

          <div className="config-diagnostics-footer">
            <span className={`config-form-message ${loggingError ? 'error' : ''}`} role="alert">
              {loggingError || ' '}
            </span>
            <div className="config-diagnostics-actions"><span className="config-card-status" role="status">{loggingSettingsDirty ? st('dirty') : ''}</span>
              <button
                type="button"
                className="secondary-button compact-button"
                disabled={controlsDisabled}
                onClick={() => void openCoreLogsDirectory()}
              >
                <FolderOpen size={16} aria-hidden="true" />
                {t('config.diagnostics.openLogs')}
              </button>
              <button
                type="button"
                className="primary-button compact-button"
                disabled={controlsDisabled || !loggingSettingsDirty}
                onClick={() => void saveCoreLoggingSettings()}
              >
                <Check size={16} aria-hidden="true" />
                {loggingSettingsBusy ? t('common.saving') : t('common.save')}
              </button>
            </div>
            {renderFeedback(loggingFeedback)}
          </div>
        </section>
<section id="config-native-software" tabIndex={-1} hidden={searching || activeSubpage !== 'software'} className="panel config-software-panel">
            <div className="config-panel-heading">
              <div className="config-heading-title">
                <Settings2 size={18} aria-hidden="true" />
                <h2>{t('config.software.title')}</h2>
              </div>
              <div className="config-heading-actions">
                {softwareStatusLabel ? (
                  <span className={`state-pill ${softwareStatusIsSaved ? 'success' : ''}`}>
                    {softwareStatusLabel}
                  </span>
                ) : null}

              </div>
            </div>
            <div className="config-software-content">
              {renderFeedback(softwareFeedback)}
              <div className="config-software-settings-list">
                <div className="config-software-setting-row">
                  <div className="config-software-setting-copy">
                    <span className="config-software-setting-icon" aria-hidden="true">
                      <Clock3 size={18} />
                    </span>
                    <div>
                      <strong>{t('config.software.autostart')}</strong>
                    </div>
                  </div>
                  <label className="switch-control" title={t('config.software.autostart')}>
                    <input
                      type="checkbox"
                      role="switch"
                      aria-label={t('config.software.autostart')}
                      checked={softwareAutostartDraft}
                      disabled={softwareSettingsLoading || softwareSettings === null || busyAction !== null}
                      onChange={(event) => {
                        setSoftwareSavedStatusVisible(false);
                        setSoftwareAutostartDraft(event.currentTarget.checked);
                      }}
                    />
                    <span className="switch-track" />
                  </label>
                </div>
                <div className="config-software-setting-row">
                  <div className="config-software-setting-copy">
                    <span className="config-software-setting-icon" aria-hidden="true">
                      <Power size={18} />
                    </span>
                    <div>
                      <strong>{t('config.software.startCoreOnLaunch')}</strong>
                    </div>
                  </div>
                  <label className="switch-control" title={t('config.software.startCoreOnLaunch')}>
                    <input
                      type="checkbox"
                      role="switch"
                      aria-label={t('config.software.startCoreOnLaunch')}
                      checked={softwareStartCoreDraft}
                      disabled={softwareSettingsLoading || softwareSettings === null || busyAction !== null}
                      onChange={(event) => {
                        setSoftwareSavedStatusVisible(false);
                        setSoftwareStartCoreDraft(event.currentTarget.checked);
                      }}
                    />
                    <span className="switch-track" />
                  </label>
                </div>
                <div className="config-software-setting-row">
                  <div className="config-software-setting-copy">
                    <span className="config-software-setting-icon" aria-hidden="true">
                      <EyeOff size={18} />
                    </span>
                    <div>
                      <span className="config-field-label"><strong>{t('config.software.silentStart')}</strong><SettingsHelp label={t('config.software.silentStart')}>{t('config.software.silentStartDescription')}</SettingsHelp></span>

                    </div>
                  </div>
                  <label className="switch-control" title={t('config.software.silentStart')}>
                    <input
                      type="checkbox"
                      role="switch"
                      aria-label={t('config.software.silentStart')}
                      checked={softwareSilentStartDraft}
                      disabled={softwareSettingsLoading || softwareSettings === null || busyAction !== null}
                      onChange={(event) => {
                        setSoftwareSavedStatusVisible(false);
                        setSoftwareSilentStartDraft(event.currentTarget.checked);
                      }}
                    />
                    <span className="switch-track" />
                  </label>
                </div>
                <div className="config-software-setting-row config-software-close-row">
                  <div className="config-software-setting-copy">
                    <span className="config-software-setting-icon" aria-hidden="true">
                      <Terminal size={18} />
                    </span>
                    <div>
                      <strong>{t('config.software.defaultTerminal')}</strong>
                    </div>
                  </div>
                  <label className="config-software-select">
                    <span className="sr-only">{t('config.software.defaultTerminal')}</span>
                    <select
                      className="config-network-input"
                      value={softwareDefaultTerminalDraft}
                      disabled={softwareSettingsLoading || softwareSettings === null || busyAction !== null}
                      onChange={(event) => {
                        setSoftwareSavedStatusVisible(false);
                        setSoftwareDefaultTerminalDraft(event.currentTarget.value);
                      }}
                    >
                      {(softwareSettings?.availableTerminals ?? []).map((terminal) => (
                        <option value={terminal.id} key={terminal.id}>
                          {terminal.id === 'auto' ? t('config.software.terminal.auto') : terminal.label}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
                <div className="config-software-setting-row config-software-close-row">
                  <div className="config-software-setting-copy">
                    <span className="config-software-setting-icon" aria-hidden="true">
                      <X size={18} />
                    </span>
                    <div>
                      <strong>{t('config.software.closeBehavior')}</strong>
                    </div>
                  </div>
                  <label className="config-software-select">
                    <span className="sr-only">{t('config.software.closeBehavior')}</span>
                    <select
                      className="config-network-input"
                      value={softwareCloseBehaviorDraft}
                      disabled={softwareSettingsLoading || softwareSettings === null || busyAction !== null}
                      onChange={(event) => {
                        setSoftwareSavedStatusVisible(false);
                        setSoftwareCloseBehaviorDraft(event.currentTarget.value as CloseBehavior);
                      }}
                    >
                      <option value="ask">{t('config.software.behavior.ask')}</option>
                      <option value="minimize-to-tray">{t('config.software.behavior.minimize')}</option>
                      <option value="exit">{t('config.software.behavior.exit')}</option>
                    </select>
                  </label>
                </div>
              </div>
            </div>

          <div className="config-settings-save-row"><button
                  type="button"
                  className="primary-button compact-button"
                  disabled={softwareSettingsLoading || softwareSettings === null || busyAction !== null || !softwareSettingsDirty}
                  onClick={() => void saveSoftwareSettings()}
                >
                  <Check size={16} aria-hidden="true" />
                  {busyAction === 'software' ? t('common.saving') : t('common.save')}
                </button></div>
        </section>
          </div>
          <div id="config-template-panel" hidden={searching || templateGroups.length === 0}><TemplateConfigSection groups={allSettingsTemplateGroups} visibleGroups={templateGroups.map(group => group.id)} onDirtyGroupsChange={setDirtyTemplateGroups} /></div>
          <div id="config-native-aliases" tabIndex={-1} role="region" aria-label={t('app.nav.thinkingAliases')} hidden={searching || activeSubpage !== 'aliases'}>{aliasesVisited ? <ThinkingAliasesPage embedded /> : null}</div>
          <div id="config-native-sensitive-words" tabIndex={-1} role="region" aria-label={t('config.sensitiveWords.title')} hidden={searching || activeSubpage !== 'requests'}>{sensitiveWordsVisited ? <SensitiveWordsPage onDirtyChange={setSensitiveWordsDirty} /> : null}</div>
        </div>
      </div>

      {addDialogOpen ? (
        <div className="config-dialog-backdrop" onMouseDown={(event) => {
          if (event.currentTarget === event.target) closeAddDialog();
        }}>
          <form
            ref={addDialogRef}
            className="config-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="add-api-key-title"
            onSubmit={(event) => void submitApiKey(event)}
          >
            <div className="config-dialog-heading">
              <div>
                <KeyRound size={19} aria-hidden="true" />
                <h2 id="add-api-key-title">
                  {editingApiKey === null ? t('config.keys.addTitle') : t('config.keys.editTitle')}
                </h2>
              </div>
              <button
                type="button"
                className="icon-button quiet"
                onClick={closeAddDialog}
                disabled={keyMutationBusy}
                title={t('common.close')}
                aria-label={t('common.close')}
              >
                <X size={18} aria-hidden="true" />
              </button>
            </div>

            <label className="config-dialog-field">
              <span>{t('config.keys.label')}</span>
              <div className="config-secret-input">
                <input
                  autoFocus
                  type={showApiKey ? 'text' : 'password'}
                  value={newApiKey}
                  onChange={(event) => {
                    setNewApiKey(event.currentTarget.value);
                    setFormError('');
                  }}
                  disabled={keyMutationBusy}
                  aria-invalid={Boolean(formError)}
                  placeholder="sk-..."
                />
                <button
                  type="button"
                  className="icon-button quiet"
                  onClick={() => setShowApiKey((visible) => !visible)}
                  disabled={keyMutationBusy}
                  title={showApiKey ? t('config.keys.hide') : t('config.keys.show')}
                  aria-label={showApiKey ? t('config.keys.hide') : t('config.keys.show')}
                >
                  {showApiKey ? (
                    <EyeOff size={17} aria-hidden="true" />
                  ) : (
                    <Eye size={17} aria-hidden="true" />
                  )}
                </button>
              </div>
            </label>

            <label className="config-dialog-field">
              <span>{t('config.keys.remark')}</span>
              <input
                className="config-dialog-text-input"
                type="text"
                value={newApiKeyRemark}
                maxLength={80}
                onChange={(event) => {
                  setNewApiKeyRemark(event.currentTarget.value);
                  setFormError('');
                }}
                disabled={keyMutationBusy}
                placeholder={t('config.keys.remarkPlaceholder')}
              />
            </label>

            <div className={`config-form-message ${formError ? 'error' : ''}`}>
              {formError || ' '}
            </div>
            {renderFeedback(keyFeedback)}

            <div className="config-dialog-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={generateApiKey}
                disabled={keyMutationBusy}
              >
                {t('config.keys.generate')}
              </button>
              <button type="submit" className="primary-button" disabled={keyMutationBusy}>
                {editingApiKey === null ? <Plus size={16} aria-hidden="true" /> : <Check size={16} aria-hidden="true" />}
                {keyMutationBusy
                  ? editingApiKey === null ? t('config.keys.adding') : t('common.saving')
                  : editingApiKey === null ? t('common.add') : t('common.save')}
              </button>
            </div>
          </form>
        </div>
      ) : null}

      {deleteIndex !== null ? (
        <div className="config-dialog-backdrop" onMouseDown={(event) => {
          if (event.currentTarget === event.target && busyAction !== 'delete-key') {
            setDeleteIndex(null);
          }
        }}>
          <div
            ref={deleteDialogRef}
            className={`config-dialog config-delete-dialog ${deletingLastKey ? 'has-warning' : ''}`}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="delete-api-key-title"
          >
            <div className="config-dialog-heading">
              <div>
                <Trash2 size={19} aria-hidden="true" />
                <h2 id="delete-api-key-title">{t('config.keys.deleteTitle')}</h2>
              </div>
            </div>
            <code className="config-delete-key">{maskApiKey(selectedDeleteKey)}</code>
            {renderFeedback(keyFeedback)}
            {deletingLastKey ? (
              <div className="config-delete-warning">
                <AlertCircle size={17} aria-hidden="true" />
                <span>{t('config.keys.deleteAllWarning')}</span>
              </div>
            ) : null}
            <div className="config-dialog-actions two-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => setDeleteIndex(null)}
                disabled={busyAction === 'delete-key'}
              >
                {t('common.cancel')}
              </button>
              <button
                type="button"
                className="danger-button"
                onClick={() => void confirmDelete()}
                disabled={busyAction === 'delete-key'}
              >
                <Trash2 size={16} aria-hidden="true" />
                {busyAction === 'delete-key' ? t('common.deleting') : t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

    </section>
  );
}

function maskApiKey(apiKey: string) {
  const value = apiKey.trim();
  if (!value) {
    return '';
  }
  const visible = value.length < 4 ? 1 : 2;
  return `${value.slice(0, visible)}${'*'.repeat(Math.max(6, 10 - visible * 2))}${value.slice(-visible)}`;
}

function routingStrategyLabel(strategy: string | undefined, t: ReturnType<typeof useI18n>['t'], locale: ReturnType<typeof useI18n>['locale']) {
  if (!strategy) {
    return t('common.loading');
  }
  const option = ROUTING_OPTIONS.find((item) => item.value === strategy);
  if (strategy === 'weighted-round-robin') return templateText(templateMessages.weighted, locale);
  return option ? t(option.labelKey) : strategy;
}
