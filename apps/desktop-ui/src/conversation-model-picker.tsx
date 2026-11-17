import { conversationModelLabel } from "@cilai/app-ui/conversation";
import { type CSSProperties, useCallback, useEffect, useState } from "react";
import type { AiModelInfo, AiReasoningEffort } from "./ai-runtime";
import { reasoningLabel } from "./dialogue-model-selection";
import { RichMenu, RichMenuEmpty, RichMenuItem, RichMenuSection } from "./rich-menu";

/** 滑块直径，同时也是滑轨高度。位置换算按它把首尾档位收进轨道两端，两处必须一致。 */
const reasoningSliderSize = 32;

function reasoningSliderPosition(index: number, count: number) {
  if (count <= 1) return "50%";
  const ratio = Math.min(1, Math.max(0, index / (count - 1)));
  const percentage = Number((ratio * 100).toFixed(4));
  const offset = Number(((0.5 - ratio) * reasoningSliderSize).toFixed(4));
  return `calc(${percentage}% + ${offset}px)`;
}

function syncReasoningThumbHover(input: HTMLInputElement, clientX: number, clientY: number) {
  const track = input.parentElement;
  if (!track) return;
  const bounds = input.getBoundingClientRect();
  const min = Number(input.min);
  const max = Number(input.max);
  const value = Number(input.value);
  const ratio = max > min ? Math.min(1, Math.max(0, (value - min) / (max - min))) : 0.5;
  const radius = reasoningSliderSize / 2;
  const centerX = bounds.left + radius + ratio * Math.max(0, bounds.width - reasoningSliderSize);
  const centerY = bounds.top + bounds.height / 2;
  track.toggleAttribute("data-thumb-hover", Math.hypot(clientX - centerX, clientY - centerY) <= radius);
}

export function adaptiveControlWidth(value: string, minimumCells: number, maximumCells: number) {
  const cells = [...value].reduce((total, character) => total + ((character.codePointAt(0) ?? 0) > 0xff ? 2 : 1), 0);
  return `${Math.min(maximumCells, Math.max(minimumCells, cells + 1))}ch`;
}

export function ConversationModelPicker(props: {
  displayModel?: AiModelInfo;
  onOpen: () => void;
  historical: boolean;
  loading: boolean;
  models: AiModelInfo[];
  selectedModelId: string | null;
  selectedConnectionId?: string | null;
  selectedReasoningEffort: AiReasoningEffort | null;
  disabled: boolean;
  onSelectModel?: (modelId: string, reasoningEffort: AiReasoningEffort | null, connectionId: string | null) => void;
}) {
  const selectedModel = props.models.find((model) => model.modelId === props.selectedModelId && model.connectionId === props.selectedConnectionId) ?? null;
  const selectedModelEfforts = selectedModel?.supportedReasoningEfforts ?? [];
  const selectedModelHasReasoning = selectedModelEfforts.length > 0;
  const [menuOpen, setMenuOpen] = useState(false);
  const [step, setStep] = useState<"model" | "reasoning">(selectedModelHasReasoning ? "reasoning" : "model");
  const [draftModelId, setDraftModelId] = useState<string | null>(selectedModelHasReasoning ? selectedModel ? JSON.stringify([selectedModel.connectionId, selectedModel.modelId]) : null : null);
  const [draftReasoningEffort, setDraftReasoningEffort] = useState<AiReasoningEffort | null>(props.selectedReasoningEffort);
  const draftModel = props.models.find((model) => JSON.stringify([model.connectionId, model.modelId]) === draftModelId) ?? null;
  const selectedModelUnavailable = Boolean(props.selectedModelId && !selectedModel);
  const selectionLabel = conversationModelLabel(undefined, "", false);
  const labelModel = selectedModel ?? props.displayModel ?? null;
  const selectedEffortInvalid = Boolean(labelModel && props.selectedReasoningEffort && !(labelModel.supportedReasoningEfforts ?? []).includes(props.selectedReasoningEffort));
  const displayedEffort = selectedEffortInvalid
    ? null
    : props.selectedReasoningEffort ?? labelModel?.defaultReasoningEffort ?? null;
  const selectedEffortLabel = selectedEffortInvalid ? "重新选择深度" : displayedEffort ? reasoningLabel(displayedEffort, labelModel) : null;
  const selectedLabel = props.loading ? "正在加载模型…" : labelModel
    ? `${labelModel.label}${selectedEffortLabel ? ` ${selectedEffortLabel}` : ""}`
    : (selectedModelUnavailable && props.historical ? "原模型已不可用" : !props.models.length ? "暂无可用模型" : null) ?? selectionLabel;
  const resetDraft = useCallback(() => {
    setStep(selectedModelHasReasoning ? "reasoning" : "model");
    setDraftModelId(selectedModelHasReasoning ? selectedModel ? JSON.stringify([selectedModel.connectionId, selectedModel.modelId]) : null : null);
    setDraftReasoningEffort(props.selectedReasoningEffort);
  }, [selectedModel?.connectionId, selectedModel?.modelId, props.selectedReasoningEffort, selectedModelHasReasoning]);
  useEffect(() => { if (!menuOpen) resetDraft(); }, [menuOpen, resetDraft]);
  const selectModel = (model: AiModelInfo) => {
    if (!props.onSelectModel) return;
    const supported = model.supportedReasoningEfforts ?? [];
    if (!supported.length) {
      props.onSelectModel(model.modelId, null, model.connectionId ?? null);
      return;
    }
    const current = model.modelId === props.selectedModelId && model.connectionId === props.selectedConnectionId ? props.selectedReasoningEffort : null;
    setDraftModelId(JSON.stringify([model.connectionId, model.modelId]));
    setDraftReasoningEffort(current);
    props.onSelectModel(model.modelId, current, model.connectionId ?? null);
    setStep("reasoning");
  };
  const completeReasoningSelection = (effort: AiReasoningEffort | null) => {
    if (!draftModel || !props.onSelectModel) return;
    setDraftReasoningEffort(effort);
    props.onSelectModel(draftModel.modelId, effort, draftModel.connectionId ?? null);
  };
  const draftReasoningTitle = draftModel?.reasoningControl === "toggle" ? "思考模式" : "思考深度";
  const draftReasoningEfforts = draftModel?.supportedReasoningEfforts ?? [];
  const draftReasoningInvalid = Boolean(draftReasoningEffort && !draftReasoningEfforts.includes(draftReasoningEffort));
  const effectiveDraftReasoningEffort = draftReasoningInvalid ? null : draftReasoningEffort ?? draftModel?.defaultReasoningEffort ?? null;
  const draftReasoningIndex = effectiveDraftReasoningEffort ? draftReasoningEfforts.indexOf(effectiveDraftReasoningEffort) : -1;
  const sliderValue = Math.max(0, draftReasoningIndex);

  return <div className="ai-starter-model-picker">
    <RichMenu
      align="end"
      ariaLabel="模型"
      className="ai-model-picker-menu"
      maxHeight={430}
      onOpenChange={(open) => { setMenuOpen(open); if (open) props.onOpen(); else resetDraft(); }}
      /* 两步共用这一个浮层：模型列表要放下名称、描述和配额标签，思考深度只有一行文字
         和一条滑轨，按同样的宽度铺开会显得空。宽度是行内样式，样式表里改不动它。 */
      width={step === "model" ? 340 : 266}
      trigger={({ open, toggle, triggerRef }) => <button aria-expanded={open} aria-haspopup="menu" aria-label={`${selectionLabel}：${selectedLabel}`} className="ch-rich-menu-trigger ai-starter-model-trigger" disabled={props.disabled} onClick={toggle} onKeyDown={(event) => { if (!open && (event.key === "ArrowDown" || event.key === "ArrowUp")) { event.preventDefault(); toggle(); } }} ref={triggerRef} type="button">
        <span style={{ width: adaptiveControlWidth(selectedLabel, 8, 30) }} title={selectedModelUnavailable ? props.selectedModelId ?? selectedLabel : selectedLabel}>{selectedLabel}</span><DownIcon/>
      </button>}>
      {() => step === "model" ? <>
        {props.models.length ? <RichMenuSection label="模型">{props.models.map((model) => {
          const checked = model.modelId === props.selectedModelId && model.connectionId === props.selectedConnectionId;
          const hasReasoning = Boolean(model.supportedReasoningEfforts?.length);
          return <RichMenuItem closeOnSelect={!hasReasoning} description={`上下文 ${(model.contextWindowTokens === null ? "未确认" : formatContextWindow(model.contextWindowTokens))}`} disabled={props.loading || !props.onSelectModel} key={`${model.connectionId}:${model.modelId}`} label={`${model.connectionName ?? model.connectionId ?? "个人连接"} · ${model.label}`} onSelect={() => selectModel(model)} selected={checked}/>;
        })}</RichMenuSection> : <RichMenuSection label="模型"><RichMenuEmpty><strong>暂无可用模型</strong>请在设置中配置模型连接。</RichMenuEmpty></RichMenuSection>}
      </> : draftModel ? <section aria-label={`选择${draftReasoningTitle}`} className="ai-reasoning-picker" role="group">
        <header>
          <span className="ai-reasoning-picker-symbol"><ReasoningIdeaIcon/></span>
          <button aria-label={`切换模型：${draftModel.label}`} className="ai-reasoning-model-switch" onClick={() => setStep("model")} type="button">
            <strong><span>{effectiveDraftReasoningEffort ? reasoningLabel(effectiveDraftReasoningEffort, draftModel) : "选择深度"}</span><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m10 7 5 5-5 5"/></svg></strong>
            <small>{draftModel.label}</small>
          </button>
          <button aria-label={draftModel.reasoningControl === "toggle" ? "恢复模型默认模式" : "恢复模型默认深度"} disabled={!draftModel.defaultReasoningEffort} onClick={() => {
            completeReasoningSelection(null);
          }} type="button"><ResetReasoningIcon/></button>
        </header>
        <div className="ai-reasoning-effort-track" style={{
          "--reasoning-slider-size": `${reasoningSliderSize}px`,
          "--selected-position": draftReasoningIndex < 0 ? `${reasoningSliderSize / 2}px` : reasoningSliderPosition(draftReasoningIndex, draftReasoningEfforts.length),
        } as CSSProperties}>
          <span aria-hidden="true" className="ai-reasoning-effort-fill"/>
          <span aria-hidden="true" className="ai-reasoning-effort-thumb"/>
          <span aria-hidden="true" className="ai-reasoning-effort-ticks">
            {draftReasoningEfforts.map((effort, index) => (
              <i
                data-selected={effectiveDraftReasoningEffort === effort}
                key={effort}
                style={{ left: reasoningSliderPosition(index, draftReasoningEfforts.length) }}/>
            ))}
          </span>
          <input
            aria-label={draftReasoningTitle}
            aria-valuetext={effectiveDraftReasoningEffort ? reasoningLabel(effectiveDraftReasoningEffort, draftModel) : "未选择"}
            disabled={!draftReasoningEfforts.length}
            max={Math.max(0, draftReasoningEfforts.length - 1)}
            min={0}
            onChange={(event) => {
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            onKeyDown={(event) => {
              if (event.key !== "Enter" && event.key !== " ") return;
              event.preventDefault();
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            onMouseLeave={(event) => event.currentTarget.parentElement?.removeAttribute("data-thumb-hover")}
            onMouseMove={(event) => syncReasoningThumbHover(event.currentTarget, event.clientX, event.clientY)}
            onKeyUp={(event) => {
              if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            onPointerUp={(event) => {
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            step={1}
            title={effectiveDraftReasoningEffort ? reasoningLabel(effectiveDraftReasoningEffort, draftModel) : "选择深度"}
            type="range"
            value={sliderValue}/>
        </div>
      </section> : null}
    </RichMenu>
  </div>;
}

function ResetReasoningIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M4 4v6h6M4.8 9A8 8 0 1 1 6 17.3"/></svg>;
}

/** 思考深度的标识。灯泡比闪电贴题：这里调的是想多深，不是跑多快。 */
function ReasoningIdeaIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24">
    <path d="M12 3a6 6 0 0 0-3.8 10.6c.7.6 1.1 1.4 1.3 2.4h5c.2-1 .6-1.8 1.3-2.4A6 6 0 0 0 12 3Z"/>
    <path d="M9.5 19h5"/>
    <path d="M10.5 21.5h3"/>
  </svg>;
}

export function formatContextWindow(tokens: number) {
  const megabytes = tokens / (1024 * 1024);
  if (megabytes >= 1) return `${Number.isInteger(megabytes) ? megabytes : megabytes.toFixed(1)}M`;
  return `${Math.round(tokens / 1024)}K`;
}

function DownIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m7 10 5 5 5-5"/></svg>; }
