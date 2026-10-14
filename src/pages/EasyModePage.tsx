import { ThemeSelector } from '../components/ThemeSelector';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  ArrowRight,
  Copy,
  Check,
  ChevronDown,
  ExternalLink,
  Languages,
  LoaderCircle,
  RefreshCw,
  X,
} from "lucide-react";
import appLogo from "../assets/logo.jpg";
import { useI18n, languageOptions, type AppLocale } from "../i18n";
import type { MessageKey } from "../i18n/resources";
import { MessageNotice, FloatingNotice, useAppNotice, type NoticeMessage } from "../appNotice";
import {
  managementApi,
  providerGroupsApi,
  readString,
  responseList,
} from "../services/managementApi";
import { appendNativeProviderGroup } from '../services/providerGroups';
import { providerRowsFromGroups, type ProviderSection } from './ApiAccessPage';
import {
  DEEPSEEK_BASE_URL,
  fetchModels,
  mergeModelOptions,
  normalizeBaseUrl,
  reconcileModelSelection,
  type ModelOption,
  type ModelProvider,
} from "../services/modelService";
import { normalizeProviderProxyUrl } from "../services/providerProxy";
import { normalizeOAuthProvider } from "../services/authFiles";
import { type ThemePreference } from "../theme";
import { AgentsPage } from "./AgentsPage";

import codexIcon from "../assets/icons/codex.svg";
import claudeIcon from "../assets/icons/claude.svg";
import antigravityIcon from "../assets/icons/antigravity.svg";
import kimiIcon from "../assets/icons/kimi-light.svg";
import grokIcon from "../assets/icons/grok.svg";
import devinIcon from "../assets/icons/devin.svg";
import metaIcon from "../assets/icons/meta.svg";
import openaiIcon from "../assets/icons/openai-light.svg";
import deepseekIcon from "../assets/icons/deepseek.svg";
import geminiIcon from "../assets/icons/gemini.svg";
import vertexIcon from '../assets/icons/vertex.svg';

type AuthMethod = "oauth" | "api";
type SetupStep = 1 | 2;

type OAuthProviderId = "codex" | "claude" | "antigravity" | "kimi" | "xai" | "devin" | "meta";

type OAuthProviderInfo = {
  id: OAuthProviderId;
  name: string;
  icon: string;
  descriptionKey: MessageKey;
};

const oauthProviders: OAuthProviderInfo[] = [
  { id: "codex", name: "Codex OAuth", icon: codexIcon, descriptionKey: "easyMode.oauth.providerDesc.codex" },
  { id: "claude", name: "Claude OAuth", icon: claudeIcon, descriptionKey: "easyMode.oauth.providerDesc.claude" },
  { id: "antigravity", name: "Antigravity OAuth", icon: antigravityIcon, descriptionKey: "easyMode.oauth.providerDesc.antigravity" },
  { id: "kimi", name: "Kimi OAuth", icon: kimiIcon, descriptionKey: "easyMode.oauth.providerDesc.kimi" },
  { id: "xai", name: "xAI OAuth", icon: grokIcon, descriptionKey: "easyMode.oauth.providerDesc.xai" },
  { id: "devin", name: "Devin OAuth", icon: devinIcon, descriptionKey: "easyMode.oauth.providerDesc.devin" },
  { id: "meta", name: "Muse (Meta) OAuth", icon: metaIcon, descriptionKey: "easyMode.oauth.providerDesc.meta" },
];

type ApiSection = "openai-compatibility" | "deepseek" | "claude" | "gemini" | "codex" | 'interactions' | 'vertex' | 'xai' | 'meta';
type ApiManagementSection = "openai-compatibility" | "claude-api-key" | "codex-api-key" | "gemini-api-key" | 'interactions-api-key' | 'vertex-api-key' | 'xai-api-key' | 'meta-api-key';

type ApiSectionOption = {
  id: ApiSection;
  managementSection: ApiManagementSection;
  nameKey: MessageKey;
  name?: string;
  provider: ModelProvider;
  defaultBaseUrl: string;
  icon: string;
};

const apiSectionOptions: ApiSectionOption[] = [
  { id: "openai-compatibility", managementSection: "openai-compatibility", nameKey: "easyMode.api.platformName.openai", provider: "openai", defaultBaseUrl: "", icon: openaiIcon },
  { id: "claude", managementSection: "claude-api-key", nameKey: "easyMode.api.platformName.claude", provider: "claude", defaultBaseUrl: "", icon: claudeIcon },
  { id: "codex", managementSection: "codex-api-key", nameKey: "easyMode.api.platformName.codex", provider: "codex", defaultBaseUrl: "", icon: codexIcon },
  { id: "gemini", managementSection: "gemini-api-key", nameKey: "easyMode.api.platformName.gemini", provider: "gemini", defaultBaseUrl: "", icon: geminiIcon },
  { id: "deepseek", managementSection: "codex-api-key", nameKey: "easyMode.api.platformName.deepseek", provider: "deepseek", defaultBaseUrl: DEEPSEEK_BASE_URL, icon: deepseekIcon },
  { id: 'interactions', managementSection: 'interactions-api-key', nameKey: 'easyMode.api.platformName.gemini', name: 'Gemini Interactions', provider: 'interactions', defaultBaseUrl: 'https://generativelanguage.googleapis.com', icon: geminiIcon },
  { id: 'vertex', managementSection: 'vertex-api-key', nameKey: 'easyMode.api.platformName.gemini', name: 'Vertex AI', provider: 'vertex', defaultBaseUrl: 'https://aiplatform.googleapis.com', icon: vertexIcon },
  { id: 'xai', managementSection: 'xai-api-key', nameKey: 'easyMode.api.platformName.openai', name: 'xAI', provider: 'xai', defaultBaseUrl: 'https://api.x.ai/v1', icon: grokIcon },
  { id: 'meta', managementSection: 'meta-api-key', nameKey: 'easyMode.api.platformName.openai', name: 'Meta', provider: 'meta', defaultBaseUrl: 'https://api.meta.ai/v1', icon: metaIcon },
];

export function EasyModePage({
  onExit,
  theme,
  setTheme,
  locale,
  setLocale,
}: {
  onExit?: () => void;
  theme?: ThemePreference;
  setTheme?: (theme: ThemePreference) => void;
  locale?: AppLocale;
  setLocale?: (locale: AppLocale) => void;
}) {
  const { t, locale: currentLocale, setLocale: setI18nLocale } = useI18n();

  const [activeStep, setActiveStep] = useState<SetupStep>(1);
  const [authMethod, setAuthMethod] = useState<AuthMethod>("oauth");

  const [authFiles, setAuthFiles] = useState<Record<string, unknown>[]>([]);
  const [apiCounts, setApiCounts] = useState<Record<ApiSection, number>>({
    interactions: 0, vertex: 0, xai: 0, meta: 0,
    "openai-compatibility": 0,
    deepseek: 0,
    claude: 0,
    gemini: 0,
    codex: 0,
  });

  const [oauthLoggingIn, setOauthLoggingIn] = useState<OAuthProviderId | null>(null);
  const [oauthAuthorization, setOauthAuthorization] = useState<{ provider: OAuthProviderId; url: string } | null>(null);
  const [oauthDeviceCode, setOauthDeviceCode] = useState<{ provider: OAuthProviderId; code: string } | null>(null);
  const oauthFeedback = useAppNotice();
  const { showNotice: showOAuthNotice, clearNotice: clearOAuthNotice } = oauthFeedback;
  const oauthPollTimer = useRef<number | null>(null);
  const oauthGeneration = useRef(0);

  const [selectedApiSection, setSelectedApiSection] = useState<ApiSection>("openai-compatibility");
  const [apiBaseUrl, setApiBaseUrl] = useState("");
  const [apiProxyUrl, setApiProxyUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [apiRemark, setApiRemark] = useState("");
  const [apiTesting, setApiTesting] = useState(false);
  const [apiTestedModels, setApiTestedModels] = useState<ModelOption[]>([]);
  const [apiSelectedModels, setApiSelectedModels] = useState<ModelOption[]>([]);
  const [apiModelsReady, setApiModelsReady] = useState(false);
  const [apiErrorMessage, setApiTestError] = useState<NoticeMessage>("");
  const apiTestError = typeof apiErrorMessage === "string"
    ? apiErrorMessage
    : t(apiErrorMessage.key, apiErrorMessage.variables);
  const [apiSaving, setApiSaving] = useState(false);
  const apiFeedback = useAppNotice();
  const { showNotice: showApiNotice, clearNotice: clearApiNotice } = apiFeedback;

  const [guideChoice, setGuideChoice] = useState<AuthMethod | null>(null);
  const [guideOAuthProvider, setGuideOAuthProvider] = useState<OAuthProviderId | null>(null);
  const [guideOAuthCompleted, setGuideOAuthCompleted] = useState(false);
  const [guideApiSaved, setGuideApiSaved] = useState(false);
  const [guideApiModelsFetched, setGuideApiModelsFetched] = useState(false);
  const [guideAgentConfigured, setGuideAgentConfigured] = useState(false);

  const [guideActive, setGuideActive] = useState(false);
  const [guideStep, setGuideStep] = useState<number>(1);
  const [spotlightRect, setSpotlightRect] = useState<{
    top: number;
    left: number;
    width: number;
    height: number;
  } | null>(null);
  const guideTooltipRef = useRef<HTMLElement | null>(null);
  const [guideCardPosition, setGuideCardPosition] = useState<{ top: number; left: number } | null>(null);
  const [langMenuOpen, setLangMenuOpen] = useState(false);
  const [themeMenuOpen, setThemeMenuOpen] = useState(false);
  const langMenuRef = useRef<HTMLDivElement>(null);
  const langButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!langMenuOpen) return undefined;
    const closeFromOutside = (event: PointerEvent) => {
      if (!langMenuRef.current?.contains(event.target as Node)) setLangMenuOpen(false);
    };
    const closeFromKeyboard = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      setLangMenuOpen(false);
      langButtonRef.current?.focus();
    };
    document.addEventListener('pointerdown', closeFromOutside);
    document.addEventListener('keydown', closeFromKeyboard);
    return () => {
      document.removeEventListener('pointerdown', closeFromOutside);
      document.removeEventListener('keydown', closeFromKeyboard);
    };
  }, [langMenuOpen]);

  const refreshSourceStatus = useCallback(async () => {
    try {
      const authFilesPayload = await managementApi.get("/credentials");
      const files = responseList(authFilesPayload, "files");
      setAuthFiles(files);

      const counts: Record<ApiSection, number> = {
        interactions: 0, vertex: 0, xai: 0, meta: 0,
        "openai-compatibility": 0,
        deepseek: 0,
        claude: 0,
        gemini: 0,
        codex: 0,
      };

      const sections = [...new Set(apiSectionOptions.map((option) => option.managementSection))];
      const recordsBySection = Object.fromEntries(await Promise.all(sections.map(async (section) => [section, await providerGroupsApi.get(section)]))) as Record<ApiManagementSection, Record<string, unknown>[]>;

      for (const option of apiSectionOptions) {
        const rows = providerRowsFromGroups(
          option.managementSection as ProviderSection,
          recordsBySection[option.managementSection],
        );
        counts[option.id] = new Set(rows.filter((row) => (
          option.id === 'deepseek' ? row.category === 'deepseek'
            : option.id === 'codex' ? row.category !== 'deepseek'
              : true
        )).map((row) => row.source?.groupIndex)).size;
      }
      setApiCounts(counts);
    } catch (e) {
      console.warn("Failed to refresh source status", e);
    }
  }, []);

  useEffect(() => {
    void refreshSourceStatus();
    return () => {
      ++oauthGeneration.current;
      if (oauthPollTimer.current !== null) window.clearTimeout(oauthPollTimer.current);
    };
  }, [refreshSourceStatus]);

  const isOAuthLoggedIn = (providerId: OAuthProviderId) => {
    const norm = providerId;
    return authFiles.some((f) => {
      const p = readString(f, "provider", "type").toLowerCase();
      if (norm === "meta") return normalizeOAuthProvider(p) === "meta";
      return (norm === "devin" && p === "cognition") || p.includes(norm) || (norm === "codex" && p.includes("openai")) || (norm === "claude" && p.includes("anthropic"));
    });
  };

  const totalLoggedInOAuth = oauthProviders.filter((p) => isOAuthLoggedIn(p.id)).length;
  const totalApiProviders = Object.values(apiCounts).reduce((a, b) => a + b, 0);
  const hasConnectedSource = totalLoggedInOAuth > 0 || totalApiProviders > 0;
  const connectedSourceCount = totalLoggedInOAuth + totalApiProviders;
  const guideConnectedSourceCount = Math.max(connectedSourceCount, guideOAuthCompleted || guideApiSaved ? 1 : 0);
  const setupStepStatus = t("easyMode.steps.current", {
    current: activeStep,
    total: 2,
  });

  const handleAuthMethodSelect = (method: AuthMethod) => {
    setAuthMethod(method);
    if (guideActive && guideStep === 1) setGuideChoice(method);
  };

  const handleStartOAuth = async (provider: OAuthProviderId) => {
    if (guideActive && guideStep === 2) {
      setGuideOAuthProvider(provider);
      setGuideOAuthCompleted(false);
    }
    const generation = ++oauthGeneration.current;
    if (oauthPollTimer.current !== null) window.clearTimeout(oauthPollTimer.current);
    oauthPollTimer.current = null;
    setOauthAuthorization(null);
    setOauthDeviceCode(null);
    setOauthLoggingIn(provider);
    clearOAuthNotice();

    try {
      const result = await invoke<{
        url?: string;
        state?: string;
        userCode?: string | null;
        flow?: string | null;
        expiresIn?: number | null;
        opened?: boolean;
        openError?: string;
      }>("start_oauth_login", {
        provider,
        browser: "default",
      });

      if (generation !== oauthGeneration.current) return;

      if (result.url) setOauthAuthorization({ provider, url: result.url });

      if (!result.state) {
        showOAuthNotice({ key: "easyMode.notice.oauthStateFailed" }, "error");
        setOauthLoggingIn(null);
        return;
      }

      if (result.userCode?.trim()) {
        setOauthDeviceCode({ provider, code: result.userCode.trim() });
      }

      if (!result.opened) {
        showOAuthNotice(result.openError
          ? { key: "oauth.openFailedDetail", variables: { error: result.openError } }
          : { key: "oauth.openFailed" }, "info");
      }

      const stateKey = result.state;
      const expirySeconds = result.expiresIn;
      const deadline = Date.now() + (typeof expirySeconds === "number" && Number.isFinite(expirySeconds) && expirySeconds > 0
        ? expirySeconds * 1000 : 10 * 60_000);
      let failures = 0;
      const poll = async () => {
        if (generation !== oauthGeneration.current) return;
        if (Date.now() >= deadline) {
          setOauthLoggingIn(null);
          setOauthAuthorization(null);
          setOauthDeviceCode((current) => current?.provider === provider ? null : current);
          showOAuthNotice({ key: "easyMode.notice.oauthTimeout" }, "error");
          return;
        }
        try {
          const pollRes = await invoke<{ status: string; error?: string }>(
            "get_oauth_status",
            { state: stateKey },
          );
          if (generation !== oauthGeneration.current) return;
          failures = 0;
          const status = (pollRes.status || "").toLowerCase();
          if (status === "ok") {
            setOauthLoggingIn(null);
            setOauthAuthorization(null);
            setOauthDeviceCode((current) => current?.provider === provider ? null : current);
            showOAuthNotice({ key: "easyMode.notice.oauthSuccess" }, "success");
            setGuideOAuthCompleted(true);
            void refreshSourceStatus();
          } else if (status === "error") {
            setOauthLoggingIn(null);
            setOauthAuthorization(null);
            setOauthDeviceCode((current) => current?.provider === provider ? null : current);
            showOAuthNotice(pollRes.error
              ? { key: "easyMode.notice.oauthFailedWithReason", variables: { error: pollRes.error } }
              : { key: "easyMode.notice.oauthFailed" }, "error");
          }
          if (status === "ok" || status === "error") return;
        } catch (error) {
          if (generation !== oauthGeneration.current) return;
          failures += 1;
          if (failures >= 3) {
            setOauthLoggingIn(null);
            setOauthAuthorization(null);
            setOauthDeviceCode((current) => current?.provider === provider ? null : current);
            showOAuthNotice(String(error), "error");
            return;
          }
        }
        if (generation === oauthGeneration.current) {
          oauthPollTimer.current = window.setTimeout(poll, Math.min(1500 * 2 ** failures, 10_000));
        }
      };
      oauthPollTimer.current = window.setTimeout(poll, 1500);
    } catch (err) {
      if (generation !== oauthGeneration.current) return;
      setOauthLoggingIn(null);
      setOauthDeviceCode((current) => current?.provider === provider ? null : current);
      showOAuthNotice(String(err), "error");
    }
  };

  const copyDeviceCode = async (code: string) => {
    try {
      await navigator.clipboard.writeText(code);
      showOAuthNotice({ key: "oauth.deviceCodeCopied" }, "success");
    } catch {
      showOAuthNotice({ key: "oauth.deviceCodeCopyFailed" }, "error");
    }
  };

  const copyAuthorizationUrl = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      showOAuthNotice({ key: "oauth.linkCopied" }, "success");
    } catch {
      showOAuthNotice({ key: "oauth.linkCopyFailed" }, "error");
    }
  };

  const openAuthorizationUrl = async (url: string) => {
    try {
      await invoke("open_oauth_url", { url, browser: "default" });
    } catch (error) {
      showOAuthNotice(String(error), "error");
    }
  };

  const handleApiSectionChange = (sec: ApiSection) => {
    setSelectedApiSection(sec);
    const opt = apiSectionOptions.find((o) => o.id === sec);
    if (opt) setApiBaseUrl(opt.defaultBaseUrl);
    setApiTestedModels([]);
    setApiSelectedModels([]);
    setApiModelsReady(false);
    setApiTestError("");
    clearApiNotice();
    setGuideApiSaved(false);
    setGuideApiModelsFetched(false);
  };

  const handleTestApi = async () => {
    if (!apiBaseUrl.trim()) {
      setApiTestError({ key: "easyMode.api.baseUrlRequired" });
      return;
    }
    if (!apiKey.trim()) {
      setApiTestError({ key: "easyMode.api.apiKeyRequired" });
      return;
    }
    setApiTesting(true);
    setApiTestError("");
    setGuideApiSaved(false);

    const opt = apiSectionOptions.find((o) => o.id === selectedApiSection);
    const providerType = opt ? opt.provider : "openai";

    try {
      const models = await fetchModels(
        providerType,
        apiBaseUrl.trim(),
        apiKey.trim(),
        undefined,
        {},
        10000,
      );
      if (models.length > 0) {
        const mergedModels = mergeModelOptions(models);
        const selectedNames = reconcileModelSelection(
          mergedModels,
          [],
          apiSelectedModels.map((model) => model.name),
          apiModelsReady ? "refresh" : "initial",
        );
        setApiTestedModels(mergedModels);
        setApiSelectedModels(
          mergedModels.filter((model) => selectedNames.has(model.name.trim().toLowerCase())),
        );
        setApiModelsReady(true);
        setGuideApiModelsFetched(true);
      } else {
        setApiTestError({ key: "easyMode.api.noModelsFound" });
      }
    } catch (err) {
      setApiTestError(String(err));
    } finally {
      setApiTesting(false);
    }
  };

  const handleToggleApiModel = (model: ModelOption) => {
    setGuideApiSaved(false);
    const key = model.name.trim().toLowerCase();
    if (!key) return;
    setApiSelectedModels((current) => {
      const selected = current.some((item) => item.name.trim().toLowerCase() === key);
      return selected
        ? current.filter((item) => item.name.trim().toLowerCase() !== key)
        : [...current, model];
    });
  };

  const handleSaveApi = async () => {
    if (!apiBaseUrl.trim()) {
      setApiTestError({ key: "easyMode.api.baseUrlRequired" });
      return;
    }
    if (!apiKey.trim()) {
      setApiTestError({ key: "easyMode.api.apiKeyRequired" });
      return;
    }
    if (selectedApiSection === "deepseek" && !apiModelsReady) {
      setApiTestError({ key: "easyMode.api.fetchListFirst" });
      return;
    }
    if (apiSelectedModels.length === 0) {
      setApiTestError({ key: "easyMode.api.modelRequired" });
      return;
    }
    if (guideActive && guideStep === 2 && authMethod === "api" && !guideApiModelsFetched) {
      setApiTestError({ key: "easyMode.api.fetchListFirst" });
      return;
    }
    let proxyUrl: string;
    try {
      proxyUrl = normalizeProviderProxyUrl(apiProxyUrl);
    } catch {
      setApiTestError({ key: "apiAccess.error.proxyUrlInvalid" });
      return;
    }
    setApiSaving(true);
    clearApiNotice();
    setApiTestError("");
    setGuideApiSaved(false);

    try {
      const selectedOption = apiSectionOptions.find((option) => option.id === selectedApiSection);
      const managementSection = selectedOption?.managementSection ?? "openai-compatibility";
      const list = await providerGroupsApi.get(managementSection);
      const selectedModels = apiSelectedModels.map((model) => ({ name: model.name.trim() }));
      const models = selectedModels;
      const newEntry = {
        name: apiRemark.trim() || (selectedApiSection === 'deepseek' ? 'DeepSeek' : selectedApiSection),
        'base-url': normalizeBaseUrl(apiBaseUrl.trim()),
        keys: [{ 'api-key': apiKey.trim(), ...(proxyUrl ? { 'proxy-url': proxyUrl } : {}) }],
        models,
      };
      await providerGroupsApi.put(managementSection, appendNativeProviderGroup(list, newEntry));
      showApiNotice({ key: "easyMode.notice.apiSaveSuccess" });
      setGuideApiSaved(true);
      void refreshSourceStatus();
    } catch (err) {
      setApiTestError(String(err));
    } finally {
      setApiSaving(false);
    }
  };

  const currentTargetId = (() => {
    if (!guideActive) return null;
    if (activeStep === 1) {
      if (guideStep === 1) return "easy-guide-choice-grid";
      if (guideStep === 2) return authMethod === "oauth" ? "easy-guide-oauth-box" : "easy-guide-api-box";
      if (guideStep >= 3) return "easy-guide-footer-action";
    } else if (activeStep === 2) {
      return "easy-guide-agents-panel";
    }
    return null;
  })();

  const updateSpotlightPosition = useCallback(() => {
    if (!guideActive || !currentTargetId) {
      setSpotlightRect(null);
      return;
    }

    const el = document.getElementById(currentTargetId);
    if (el) {
      const rect = el.getBoundingClientRect();
      const next = {
        top: Math.max(0, rect.top),
        left: Math.max(0, rect.left),
        width: rect.width,
        height: rect.height,
      };
      setSpotlightRect((previous) => previous && previous.top === next.top && previous.left === next.left
        && previous.width === next.width && previous.height === next.height ? previous : next);
    } else {
      setSpotlightRect(null);
    }
  }, [guideActive, currentTargetId]);

  useEffect(() => {
    let frame: number | null = null;
    const schedulePosition = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(() => {
        frame = null;
        updateSpotlightPosition();
      });
    };
    const timer = setTimeout(() => {
      if (currentTargetId) document.getElementById(currentTargetId)?.scrollIntoView({ behavior: "smooth", block: "nearest" });
      schedulePosition();
    }, 100);
    window.addEventListener("resize", schedulePosition);
    window.addEventListener("scroll", schedulePosition, { capture: true, passive: true });
    return () => {
      clearTimeout(timer);
      if (frame !== null) window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", schedulePosition);
      window.removeEventListener("scroll", schedulePosition, true);
    };
  }, [currentTargetId, updateSpotlightPosition]);

  useEffect(() => {
    setGuideCardPosition(null);
  }, [currentTargetId]);

  useLayoutEffect(() => {
    if (!guideActive || !spotlightRect || !guideTooltipRef.current) {
      setGuideCardPosition(null);
      return;
    }

    const cardRect = guideTooltipRef.current.getBoundingClientRect();
    const viewportPadding = 16;
    const gap = 18;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const cardWidth = cardRect.width;
    const cardHeight = cardRect.height;
    const targetTop = spotlightRect.top;
    const targetBottom = spotlightRect.top + spotlightRect.height;
    const targetRight = spotlightRect.left + spotlightRect.width;
    const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), Math.max(min, max));
    const centeredLeft = clamp(spotlightRect.left, viewportPadding, viewportWidth - cardWidth - viewportPadding);
    const belowTop = targetBottom + gap;
    const aboveTop = targetTop - cardHeight - gap;
    let top: number;
    let left = centeredLeft;

    if (belowTop + cardHeight <= viewportHeight - viewportPadding) {
      top = belowTop;
    } else if (aboveTop >= viewportPadding) {
      top = aboveTop;
    } else if (targetRight + gap + cardWidth <= viewportWidth - viewportPadding) {
      left = targetRight + gap;
      top = clamp(targetTop, viewportPadding, viewportHeight - cardHeight - viewportPadding);
    } else if (spotlightRect.left - gap - cardWidth >= viewportPadding) {
      left = spotlightRect.left - cardWidth - gap;
      top = clamp(targetTop, viewportPadding, viewportHeight - cardHeight - viewportPadding);
    } else {
      top = clamp(belowTop, viewportPadding, viewportHeight - cardHeight - viewportPadding);
    }

    setGuideCardPosition({ top, left });
  }, [
    authMethod,
    currentTargetId,
    guideActive,
    guideAgentConfigured,
    guideApiSaved,
    guideOAuthCompleted,
    guideStep,
    spotlightRect,
  ]);

  const guideCanAdvance = guideStep === 1
    ? guideChoice !== null
    : guideStep === 2
      ? authMethod === "oauth"
        ? guideOAuthProvider !== null && guideOAuthCompleted
        : guideApiSaved
      : guideStep === 3
        ? hasConnectedSource || guideOAuthCompleted || guideApiSaved
        : guideAgentConfigured;

  const handleNextGuideStep = () => {
    if (!guideCanAdvance) return;
    if (guideStep === 1) {
      setGuideStep(2);
    } else if (guideStep === 2) {
      setGuideStep(3);
    } else if (guideStep === 3) {
      setActiveStep(2);
      setGuideStep(4);
    } else if (guideStep === 4) {
      setGuideActive(false);
    }
  };

  const handlePrevGuideStep = () => {
    if (guideStep === 4) {
      setActiveStep(1);
      setGuideStep(3);
    } else if (guideStep > 1) {
      setGuideStep(guideStep - 1);
    }
  };

  const handleGuideToggle = () => {
    if (guideActive) {
      setGuideActive(false);
      return;
    }
    setActiveStep(1);
    setGuideStep(1);
    setGuideChoice(null);
    setGuideOAuthProvider(null);
    setGuideOAuthCompleted(false);
    setGuideApiSaved(false);
    setGuideApiModelsFetched(false);
    setGuideAgentConfigured(false);
    setGuideActive(true);
  };

  const currentActiveLang = languageOptions.find((opt) => opt.value === (locale || currentLocale)) || languageOptions[0];
  const handleLanguageListKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const options = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="option"]'));
    if (!options.length) return;
    const current = options.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? options.length - 1
        : event.key === 'ArrowDown'
          ? (Math.max(current, -1) + 1) % options.length
          : (current <= 0 ? options.length : current) - 1;
    event.preventDefault();
    options[next]?.focus();
  };

  return (
    <section className="page simple-mode-page simple-mode-expanded">
      <h1 className="sr-only">{t('app.nav.easy')}</h1>
      {guideActive ? <div className="guide-dimmed-overlay" /> : null}

      <header className="simple-mode-topbar">
        <div className="simple-mode-topbar-left">
          <div className="simple-mode-brand">
            <img src={appLogo} alt="" className="brand-mark brand-logo" />
            <div className="simple-mode-brand-text">
              <div className="simple-mode-brand-title">
                <strong>EasyCLIProxyAPI</strong>
                <span className="simple-mode-badge">{t("easyMode.badge")}</span>
              </div>
              <span className="simple-mode-brand-sub">{t("easyMode.brandSub")}</span>
            </div>
          </div>
          <button
            type="button"
            className={`simple-mode-guide-toggle simple-mode-highlight-button${guideActive ? " active" : ""}`}
            title={guideActive ? t("easyMode.guide.toggleClose") : t("easyMode.guide.toggleOpen")}
            onClick={() => {
              handleGuideToggle();
            }}
          >
            <span>{guideActive ? t("easyMode.guide.buttonRunning") : t("easyMode.guide.button")}</span>
          </button>
        </div>

        <div className="simple-mode-topbar-right">
          {setTheme ? (
            <ThemeSelector value={theme ?? 'light'} onChange={setTheme} open={themeMenuOpen}
              onOpenChange={(open) => { setThemeMenuOpen(open); if (open) setLangMenuOpen(false); }} />
          ) : null}

          <div ref={langMenuRef} className="simple-mode-lang-dropdown">
            <button
              ref={langButtonRef}
              type="button"
              className="simple-mode-lang-btn"
              onClick={() => { setThemeMenuOpen(false); setLangMenuOpen(!langMenuOpen); }}
              title={t("easyMode.language.switch")}
              aria-label={t("easyMode.language.switch")}
              aria-haspopup="listbox"
              aria-expanded={langMenuOpen}
              aria-controls="simple-mode-language-list"
              onKeyDown={(event) => {
                if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
                event.preventDefault();
                setThemeMenuOpen(false);
                setLangMenuOpen(true);
                window.requestAnimationFrame(() => {
                  const options = langMenuRef.current?.querySelectorAll<HTMLButtonElement>('[role="option"]');
                  options?.[event.key === 'ArrowUp' ? options.length - 1 : 0]?.focus();
                });
              }}
            >
              <Languages size={15} />
              <span>{currentActiveLang.nativeLabel}</span>
              <ChevronDown size={13} />
            </button>
            {langMenuOpen ? (
              <div id="simple-mode-language-list" className="simple-mode-lang-menu" data-theme-surface="overlay" role="listbox" aria-label={t("easyMode.language.switch")} onKeyDown={handleLanguageListKeyDown}>
                {languageOptions.map((opt) => (
                  <button
                    key={opt.value}
                    type="button"
                    className={opt.value === (locale || currentLocale) ? "selected" : ""}
                    role="option"
                    aria-selected={opt.value === (locale || currentLocale)}
                    onClick={() => {
                      if (setLocale) setLocale(opt.value);
                      else setI18nLocale(opt.value);
                      setLangMenuOpen(false);
                      window.requestAnimationFrame(() => langButtonRef.current?.focus());
                    }}
                  >
                    <span>{opt.nativeLabel}</span>
                    {opt.value === (locale || currentLocale) ? <Check size={14} /> : null}
                  </button>
                ))}
              </div>
            ) : null}
          </div>

          <button
            type="button"
            className="secondary-button simple-mode-exit-btn"
            title={t("easyMode.exitTitle")}
            onClick={() => onExit?.()}
          >
            <span>{t("easyMode.exit")}</span>
          </button>
        </div>
      </header>

      <nav className="simple-mode-step-status" aria-label={t("easyMode.steps.label")}>
        <div className="simple-mode-step-status-heading">
          <span>{t("easyMode.steps.label")}</span>
          <strong>{setupStepStatus}</strong>
        </div>
        <div className="simple-mode-step-status-track">
          <button
            type="button"
            className={`simple-mode-step-status-item${activeStep === 1 ? " active" : " complete"}`}
            aria-current={activeStep === 1 ? "step" : undefined}
            aria-pressed={activeStep === 1}
            disabled={guideActive}
            onClick={() => setActiveStep(1)}
          >
            <span className="simple-mode-step-status-number">
              {activeStep > 1 ? <Check size={14} aria-hidden="true" /> : "1"}
            </span>
            <span className="simple-mode-step-status-copy">
              <strong>{t("easyMode.overview.providerTitle")}</strong>
              <small>{t("easyMode.steps.step1Description")}</small>
            </span>
          </button>
          <span className={`simple-mode-step-status-connector${activeStep > 1 ? " complete" : ""}`} aria-hidden="true" />
          <button
            type="button"
            className={`simple-mode-step-status-item${activeStep === 2 ? " active" : ""}`}
            aria-current={activeStep === 2 ? "step" : undefined}
            aria-pressed={activeStep === 2}
            disabled={guideActive || !hasConnectedSource}
            onClick={() => setActiveStep(2)}
          >
            <span className="simple-mode-step-status-number">2</span>
            <span className="simple-mode-step-status-copy">
              <strong>{t("easyMode.overview.agentTitle")}</strong>
              <small>{t("easyMode.steps.step2Description")}</small>
            </span>
          </button>
        </div>
      </nav>

      {activeStep === 1 ? (
        <section className="panel simple-mode-task">
          <div className="simple-mode-task-heading">
            <div>
              <h2>{t("easyMode.overview.providerTitle")}</h2>
              <p>{t("easyMode.setup.providerDescription")}</p>
            </div>
          </div>

          <div
            id="easy-guide-choice-grid"
            className={`simple-mode-choice-grid${guideActive && guideStep === 1 ? " guide-focus-highlight" : ""}`}
          >
            <button
              type="button"
              className={`simple-mode-choice${authMethod === "oauth" ? " selected" : ""}`}
              aria-pressed={authMethod === "oauth"}
              onClick={() => handleAuthMethodSelect("oauth")}
              disabled={guideActive && guideStep !== 1}
            >
              <div className="simple-mode-choice-heading">
                <div className="simple-mode-choice-title">
                  <strong>{t("easyMode.oauth.title")}</strong>
                  {totalLoggedInOAuth > 0 ? (
                    <span className="state-pill success">
                      {t("easyMode.oauth.accountsLoggedIn", { count: totalLoggedInOAuth })}
                    </span>
                  ) : (
                    <span className="state-pill neutral">{t("easyMode.oauth.recommended")}</span>
                  )}
                </div>
              </div>
              <span>{t("easyMode.oauth.description")}</span>
              <small>{t("easyMode.oauth.supportedProviders")}</small>
            </button>

            <button
              type="button"
              className={`simple-mode-choice${authMethod === "api" ? " selected" : ""}`}
              aria-pressed={authMethod === "api"}
              onClick={() => handleAuthMethodSelect("api")}
              disabled={guideActive && guideStep !== 1}
            >
              <div className="simple-mode-choice-heading">
                <div className="simple-mode-choice-title">
                  <strong>{t("easyMode.api.title")}</strong>
                  {totalApiProviders > 0 ? (
                    <span className="state-pill success">
                      {t("easyMode.api.platformsConnected", { count: totalApiProviders })}
                    </span>
                  ) : null}
                </div>
              </div>
              <span>{t("easyMode.api.description")}</span>
              <small>{t("easyMode.api.supportedPlatforms")}</small>
            </button>
          </div>

          {authMethod === "oauth" ? (
            <div
              id="easy-guide-oauth-box"
              className={`simple-mode-embedded-box${guideActive && guideStep === 2 ? " guide-focus-highlight" : ""}`}
            >
              <FloatingNotice key={oauthFeedback.revision} notice={oauthFeedback.notice} onDismiss={clearOAuthNotice} />
              <div className="simple-mode-provider-grid">
                {oauthProviders.map((provider) => {
                  const loggedIn = isOAuthLoggedIn(provider.id);
                  const isLogging = oauthLoggingIn === provider.id;

                  return (
                    <article
                      key={provider.id}
                      className={`panel simple-mode-provider-card${loggedIn ? " connected" : ""}`}
                    >
                      <div className="simple-mode-provider-card-head">
                        <div className="simple-mode-provider-logo">
                          <img src={provider.icon} alt="" className={provider.id === "devin" ? "devin-logo" : undefined} />
                        </div>
                        <div className="simple-mode-provider-copy">
                          <strong>{provider.name}</strong>
                          <span>{t(provider.descriptionKey)}</span>
                        </div>
                      </div>

                      {oauthAuthorization?.provider === provider.id ? (
                        <div className="simple-mode-oauth-authorization">
                          {provider.id === "meta" ? <p className="oauth-hint">{t("oauth.metaHint")}</p> : null}
                          <div className="oauth-auth-url-box">
                            <div className="oauth-auth-url-label">{t("oauth.authorizationLink")}</div>
                            <div className="oauth-auth-url-value" title={oauthAuthorization.url}>{oauthAuthorization.url}</div>
                            <div className="oauth-auth-url-actions">
                              <button type="button" className="secondary-button compact-button" onClick={() => void copyAuthorizationUrl(oauthAuthorization.url)}>
                                <Copy size={14} aria-hidden="true" />{t("oauth.copyLink")}
                              </button>
                              <button type="button" className="secondary-button compact-button" onClick={() => void openAuthorizationUrl(oauthAuthorization.url)}>
                                <ExternalLink size={14} aria-hidden="true" />{t("oauth.openLink")}
                              </button>
                            </div>
                          </div>
                          {oauthDeviceCode?.provider === provider.id ? (
                            <div className="simple-mode-device-code">
                              <div className="simple-mode-device-code-label">{t("oauth.deviceCodeLabel")}</div>
                              <code>{oauthDeviceCode.code}</code>
                              <button
                                type="button"
                                className="secondary-button compact-button"
                                onClick={() => void copyDeviceCode(oauthDeviceCode.code)}
                              >
                                <Copy size={14} aria-hidden="true" />
                                {t("oauth.copyDeviceCode")}
                              </button>
                            </div>
                          ) : null}
                        </div>
                      ) : null}

                      <div className="simple-mode-provider-card-foot">
                        {loggedIn ? (
                          <span className="state-pill success">
                            <Check size={12} style={{ marginRight: 4 }} />
                            {t("easyMode.oauth.loggedIn")}
                          </span>
                        ) : (
                          <span className="state-pill neutral">{t("easyMode.status.notLoggedIn")}</span>
                        )}

                        <button
                          type="button"
                          className={loggedIn ? "secondary-button" : "primary-button"}
                          disabled={isLogging}
                          onClick={() => void handleStartOAuth(provider.id)}
                        >
                          {isLogging ? (
                            <>
                              <LoaderCircle size={14} className="spin" style={{ marginRight: 6 }} />
                              {t("easyMode.oauth.loggingIn")}
                            </>
                          ) : loggedIn ? (
                            t("easyMode.oauth.relogin")
                          ) : (
                            t("oauth.startLogin")
                          )}
                        </button>
                      </div>
                    </article>
                  );
                })}
              </div>
            </div>
          ) : null}

          {authMethod === "api" ? (
            <div
              id="easy-guide-api-box"
              className={`simple-mode-embedded-box${guideActive && guideStep === 2 ? " guide-focus-highlight" : ""}`}
            >
              <FloatingNotice key={apiFeedback.revision} notice={apiFeedback.notice} onDismiss={clearApiNotice} />
              {apiTestError ? (
                <MessageNotice message={apiTestError} onDismiss={() => setApiTestError("")} />
              ) : null}

              <div className="simple-mode-api-form">
                <div className="simple-mode-api-platforms">
                  {apiSectionOptions.map((opt) => (
                    <button
                      type="button"
                      key={opt.id}
                      className={`secondary-button simple-mode-api-platform${selectedApiSection === opt.id ? " active" : ""}`}
                      onClick={() => handleApiSectionChange(opt.id)}
                    >
                      <img className="simple-mode-api-platform-icon" src={opt.icon} alt="" />
                      {opt.name ?? t(opt.nameKey)}
                    </button>
                  ))}
                </div>

                <div className="simple-mode-field">
                  <label>{t("easyMode.api.name")}</label>
                  <input
                    type="text"
                    className="text-input"
                    value={apiRemark}
                    onChange={(e) => { setApiRemark(e.target.value); setGuideApiSaved(false); }}
                    placeholder={t("easyMode.api.namePlaceholder")}
                  />
                </div>

                <div className="simple-mode-api-fields">
                  <div className="simple-mode-field">
                    <label>{t("easyMode.api.baseUrl")}</label>
                    <input
                      type="text"
                      className="text-input"
                      value={apiBaseUrl}
                      onChange={(e) => {
                        setApiBaseUrl(e.target.value);
                        setApiModelsReady(false);
                        setGuideApiModelsFetched(false);
                        setGuideApiSaved(false);
                      }}
                      placeholder="https://..."
                    />
                  </div>
                  <div className="simple-mode-field">
                    <label>{t("easyMode.api.apiKey")}</label>
                    <input
                      type="password"
                      className="text-input"
                      value={apiKey}
                      onChange={(e) => {
                        setApiKey(e.target.value);
                        setApiModelsReady(false);
                        setGuideApiModelsFetched(false);
                        setGuideApiSaved(false);
                      }}
                      placeholder="sk-..."
                    />
                  </div>
                </div>

                <div className="simple-mode-field">
                  <label>{t("apiAccess.field.proxyUrl")}</label>
                  <input
                    type="text"
                    className="text-input"
                    value={apiProxyUrl}
                    onChange={(event) => { setApiProxyUrl(event.target.value); setGuideApiSaved(false); }}
                    placeholder="socks5://127.0.0.1:1080"
                  />
                </div>

                <div className="simple-mode-api-model-card">
                  {apiTestedModels.length === 0 ? (
                    <div className="simple-mode-api-model-fetch">
                      <button
                        type="button"
                        className="secondary-button simple-mode-api-fetch-button"
                        disabled={apiTesting || !apiBaseUrl.trim() || !apiKey.trim()}
                        onClick={() => void handleTestApi()}
                      >
                        {apiTesting ? (
                          <>
                            <LoaderCircle size={14} className="spin" style={{ marginRight: 6 }} />
                            {t("easyMode.api.fetchingModels")}
                          </>
                        ) : (
                          t("easyMode.api.fetchModels")
                        )}
                      </button>
                    </div>
                  ) : (
                    <>
                      <div className="simple-mode-api-model-heading">
                        <strong>{t("easyMode.api.modelListTitle")}</strong>
                        <button
                          type="button"
                          className="secondary-button compact-button"
                          disabled={apiTesting || !apiBaseUrl.trim() || !apiKey.trim()}
                          onClick={() => void handleTestApi()}
                        >
                          <RefreshCw size={14} className={apiTesting ? "spin" : ""} />
                          {t("common.refresh")}
                        </button>
                      </div>

                      <div className="simple-mode-api-model-selection">
                        <div className="simple-mode-api-model-selection-heading">
                          <span>
                            {selectedApiSection === "deepseek" && !apiModelsReady
                              ? t("easyMode.api.modelListStale")
                              : t("easyMode.api.modelListHint")}
                          </span>
                        </div>
                        <div className="simple-mode-api-model-options">
                          {apiTestedModels.map((model) => {
                            const key = model.name.trim().toLowerCase();
                            const selected = apiSelectedModels.some(
                              (item) => item.name.trim().toLowerCase() === key,
                            );
                            return (
                              <label
                                className={`simple-mode-api-model-option${selected ? " selected" : ""}`}
                                key={model.name}
                              >
                                <input
                                  type="checkbox"
                                  checked={selected}
                                  disabled={apiTesting}
                                  onChange={() => handleToggleApiModel(model)}
                                />
                                <span title={model.name}>{model.name}</span>
                              </label>
                            );
                          })}
                        </div>
                      </div>

                      <div className="simple-mode-api-actions">
                        <button
                          type="button"
                          className="primary-button"
                          disabled={
                            apiSaving
                            || !apiBaseUrl.trim()
                            || !apiKey.trim()
                            || apiSelectedModels.length === 0
                            || (selectedApiSection === "deepseek" && !apiModelsReady)
                          }
                          onClick={() => void handleSaveApi()}
                        >
                          {t("easyMode.api.saveAndConnect")}
                        </button>
                      </div>
                    </>
                  )}
                </div>
              </div>
            </div>
          ) : null}

          <div
            id="easy-guide-footer-action"
            className={`simple-mode-task-footer${guideActive && guideStep === 3 ? " guide-focus-highlight" : ""}`}
            style={{ marginTop: 10 }}
          >
            <div className="simple-mode-selection">
              {hasConnectedSource ? (
                <span style={{ color: "var(--ui-accent-strong)", fontWeight: 600 }}>
                  {t("easyMode.status.connectedModels", { count: connectedSourceCount })}
                </span>
              ) : (
                <span className="muted">{t("easyMode.status.noModelsYet")}</span>
              )}
            </div>

            <button
              type="button"
              className="primary-button"
              style={{ minHeight: 42, padding: "0 22px" }}
              disabled={!hasConnectedSource}
              onClick={() => {
                setActiveStep(2);
                if (guideActive) setGuideStep(4);
              }}
            >
              {t("easyMode.navigation.nextAgent")}
              <ArrowRight size={16} style={{ marginLeft: 6 }} />
            </button>
          </div>
        </section>
      ) : null}

      {activeStep === 2 ? (
        <section
          id="easy-guide-agents-panel"
          className={`panel simple-mode-task${guideActive && guideStep === 4 ? " guide-focus-highlight" : ""}`}
        >
          <AgentsPage embedded onConfigurationApplied={() => setGuideAgentConfigured(true)} />

          <div className="simple-mode-task-footer" style={{ marginTop: 14 }}>
            <button
              type="button"
              className="secondary-button"
              onClick={() => {
                setActiveStep(1);
                if (guideActive) setGuideStep(3);
              }}
            >
              <ArrowLeft size={16} style={{ marginRight: 6 }} />
              {t("easyMode.navigation.back")}
            </button>

          </div>
        </section>
      ) : null}

      {guideActive && spotlightRect ? (
        <aside
          ref={guideTooltipRef}
          className="guide-interactive-card"
          data-theme-surface="overlay"
          style={{
            top: guideCardPosition ? `${guideCardPosition.top}px` : "0px",
            left: guideCardPosition ? `${guideCardPosition.left}px` : "0px",
            visibility: guideCardPosition ? "visible" : "hidden",
          }}
        >
          <div className="guide-tooltip-header">
            <div className="guide-tooltip-badge">
              <span>
                {guideStep === 1 && t("easyMode.guide.badgeStep1")}
                {guideStep === 2 && (authMethod === "oauth" ? t("easyMode.guide.badgeStep2OAuth") : t("easyMode.guide.badgeStep2Api"))}
                {guideStep === 3 && t("easyMode.guide.badgeStep3")}
                {guideStep === 4 && t("easyMode.guide.badgeStep4")}
              </span>
            </div>

            <button
              type="button"
              className="guide-tooltip-close"
              title={t("easyMode.guide.close")}
              onClick={() => setGuideActive(false)}
            >
              <X size={15} />
            </button>
          </div>

          <div className="guide-tooltip-body">
            {guideStep === 1 ? (
              <>
                <h4>{t("easyMode.guide.cardStep1Title")}</h4>
                <p>
                  <strong>{t("easyMode.guide.cardStep1OAuth")}</strong>{t("easyMode.guide.cardStep1OAuthDesc")}<br />
                  <strong>{t("easyMode.guide.cardStep1Api")}</strong>{t("easyMode.guide.cardStep1ApiDesc")}
                </p>
                <div className="guide-tooltip-tip">
                  {guideChoice ? t("easyMode.guide.cardStep1TipSelected") : t("easyMode.guide.cardStep1TipUnselected")}
                </div>
              </>
            ) : null}

            {guideStep === 2 && authMethod === "oauth" ? (
              <>
                <h4>{t("easyMode.guide.cardStep2OAuthTitle")}</h4>
                <p>
                  {t("easyMode.guide.oauthInstructions", { start: t("oauth.startLogin"), relogin: t("easyMode.oauth.relogin") })}<br />
                  {t("easyMode.guide.oauthCompletion", { signedIn: t("easyMode.oauth.loggedIn") })}
                </p>
                <div className="guide-tooltip-tip">
                  {guideOAuthCompleted && guideOAuthProvider ? (
                    <strong>{t("easyMode.guide.cardStep2OAuthSuccessTip", { provider: oauthProviders.find((provider) => provider.id === guideOAuthProvider)?.name ?? "" })}</strong>
                  ) : (
                    t("easyMode.guide.cardStep2OAuthWaitTip")
                  )}
                </div>
              </>
            ) : null}

            {guideStep === 2 && authMethod === "api" ? (
              <>
                <h4>{t("easyMode.guide.cardStep2ApiTitle")}</h4>
                <p>
                  {t("easyMode.guide.cardStep2Api1")}<br />
                  {t("easyMode.guide.apiCredentials", { baseUrl: t("easyMode.api.baseUrl"), apiKey: t("easyMode.api.apiKey") })}<br />
                  {t("easyMode.guide.apiModels", { fetch: t("easyMode.api.fetchModels"), save: t("easyMode.api.saveAndConnect") })}
                </p>
                <div className="guide-tooltip-tip">
                  {guideApiSaved ? (
                    <strong>{t("easyMode.guide.cardStep2ApiSavedTip")}</strong>
                  ) : !apiBaseUrl.trim() || !apiKey.trim() ? (
                    t("easyMode.guide.cardStep2ApiFillTip")
                  ) : !guideApiModelsFetched || apiTestedModels.length === 0 ? (
                    t("easyMode.guide.cardStep2ApiFetchTip")
                  ) : apiSelectedModels.length === 0 ? (
                    t("easyMode.guide.cardStep2ApiSelectTip")
                  ) : (
                    <span>{t("easyMode.guide.cardStep2ApiSelectedTip", { count: apiSelectedModels.length })}</span>
                  )}
                </div>
              </>
            ) : null}

            {guideStep === 3 ? (
              <>
                <h4>{t("easyMode.guide.cardStep3Title")}</h4>
                <p>
                  {t("easyMode.guide.sourcesSummary", { count: guideConnectedSourceCount })}<br />
                  {t("easyMode.guide.sourcesNext", { next: t("easyMode.navigation.nextAgent") })}
                </p>
                <div className="guide-tooltip-tip">
                  {guideCanAdvance ? t("easyMode.guide.cardStep3TipReady") : t("easyMode.guide.cardStep3TipPending")}
                </div>
              </>
            ) : null}

            {guideStep === 4 ? (
              <>
                <h4>{t("easyMode.guide.cardStep4Title")}</h4>
                <p>
                  {t("easyMode.guide.clientSelection")}<br />
                  {t("easyMode.guide.clientApply")}
                </p>
                <div className="guide-tooltip-tip">
                  {guideAgentConfigured ? t("easyMode.guide.cardStep4TipConfigured") : t("easyMode.guide.cardStep4TipPending")}
                </div>
              </>
            ) : null}
          </div>

          <div className="guide-tooltip-footer">
            <div className="guide-tooltip-step-dots">
              {[1, 2, 3, 4].map((step) => (
                <span
                  key={step}
                  className={`guide-step-dot${guideStep === step ? " active" : ""}`}
                  aria-hidden="true"
                />
              ))}
            </div>

            <div className="guide-tooltip-actions">
              {guideStep > 1 ? (
                <button
                  type="button"
                  className="secondary-button guide-btn-sm"
                  onClick={handlePrevGuideStep}
                >
                  {t("easyMode.navigation.back")}
                </button>
              ) : null}

              <button
                type="button"
                className="primary-button guide-btn-sm"
                onClick={handleNextGuideStep}
                disabled={!guideCanAdvance}
              >
                {guideStep === 4 ? t("easyMode.guide.finish") : t("easyMode.navigation.next")}
              </button>
            </div>
          </div>
        </aside>
      ) : null}
    </section>
  );
}
