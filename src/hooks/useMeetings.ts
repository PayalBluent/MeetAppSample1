import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api } from "@/lib/api";
import { qk } from "@/lib/query";
import type { CaptureMode, Meeting, RecorderStatus, Settings } from "@/types";

export function useMeetings() {
  return useQuery({ queryKey: qk.meetings, queryFn: api.listMeetings });
}

export function useMeeting(id: string | undefined) {
  return useQuery({
    queryKey: qk.meeting(id ?? "none"),
    queryFn: () => api.getMeeting(id!),
    enabled: !!id,
  });
}

export function useSettings() {
  return useQuery({ queryKey: qk.settings, queryFn: api.getSettings });
}

export function useRecorderStatus() {
  return useQuery({
    queryKey: qk.recorderStatus,
    queryFn: api.getRecorderStatus,
  });
}

export function useDetectedMeetings() {
  return useQuery({ queryKey: qk.detected, queryFn: api.getDetectedMeetings });
}

export function useUpdateSettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: Partial<Settings>) => api.updateSettings(patch),
    // Optimistic: settings toggles should feel instant.
    onMutate: async (patch) => {
      await qc.cancelQueries({ queryKey: qk.settings });
      const prev = qc.getQueryData<Settings>(qk.settings);
      if (prev) qc.setQueryData(qk.settings, { ...prev, ...patch });
      return { prev };
    },
    onError: (_e, _patch, ctx) => {
      if (ctx?.prev) qc.setQueryData(qk.settings, ctx.prev);
    },
    onSuccess: (next) => qc.setQueryData(qk.settings, next),
  });
}

export function useSetMode() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (mode: CaptureMode) => api.setMode(mode),
    onSuccess: (status) => {
      qc.setQueryData(qk.recorderStatus, status);
      qc.invalidateQueries({ queryKey: qk.settings });
      // Switching to Off clears detections on the backend; refresh so the panel
      // drops any lingering "meeting detected" cards immediately.
      qc.invalidateQueries({ queryKey: qk.detected });
    },
  });
}

export function useStartCapture() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.startCapture,
    onSuccess: (status) => {
      qc.setQueryData(qk.recorderStatus, status);
      qc.invalidateQueries({ queryKey: qk.meetings });
    },
  });
}

export function useStopCapture() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.stopCapture,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.recorderStatus });
      qc.invalidateQueries({ queryKey: qk.meetings });
    },
  });
}

export function useSetInputGain() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (gain: number) => api.setInputGain(gain),
    // Optimistic: reflect the new gain in the meter/label immediately so the
    // volume control feels instant, then reconcile with the server's clamp.
    onMutate: (gain) => {
      const prev = qc.getQueryData<RecorderStatus>(qk.recorderStatus);
      if (prev) qc.setQueryData(qk.recorderStatus, { ...prev, inputGain: gain });
      return { prev };
    },
    onError: (_e, _gain, ctx) => {
      if (ctx?.prev) qc.setQueryData(qk.recorderStatus, ctx.prev);
    },
    onSuccess: (status) => qc.setQueryData(qk.recorderStatus, status),
  });
}

export function useCaptureDetected() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.captureDetected(id),
    onSuccess: (status) => {
      qc.setQueryData(qk.recorderStatus, status);
      qc.invalidateQueries({ queryKey: qk.detected });
    },
  });
}

export function useDismissDetected() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.dismissDetected(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.detected }),
  });
}

export function useToggleFlag() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      flag,
    }: {
      id: string;
      flag: "locked" | "starred" | "bookmarked";
    }) => api.toggleFlag(id, flag),
    onSuccess: (meeting) => {
      qc.setQueryData(qk.meeting(meeting.id), meeting);
      qc.invalidateQueries({ queryKey: qk.meetings });
    },
  });
}

export function useRenameMeeting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      api.renameMeeting(id, title),
    onSuccess: (meeting) => {
      qc.setQueryData(qk.meeting(meeting.id), meeting);
      qc.invalidateQueries({ queryKey: qk.meetings });
    },
  });
}

export function useUpdateActionItem() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      meetingId,
      itemId,
      done,
    }: {
      meetingId: string;
      itemId: string;
      done: boolean;
    }) => api.updateActionItem(meetingId, itemId, done),
    onSuccess: (meeting) => qc.setQueryData(qk.meeting(meeting.id), meeting),
  });
}

export function useDeleteMeeting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteMeeting(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.meetings }),
  });
}

export function useTranscribeMeeting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.transcribeMeeting(id),
    onSuccess: (meeting: Meeting) => {
      qc.setQueryData(qk.meeting(meeting.id), meeting);
      qc.invalidateQueries({ queryKey: qk.meetings });
    },
  });
}

export function useSummarizeMeeting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.summarizeMeeting(id),
    onSuccess: (meeting: Meeting) =>
      qc.setQueryData(qk.meeting(meeting.id), meeting),
  });
}

export function useEnhanceAudio() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.enhanceMeetingAudio(id),
    onSuccess: (meeting: Meeting) =>
      qc.setQueryData(qk.meeting(meeting.id), meeting),
  });
}

export function useCleanAudio() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.cleanMeetingAudio(id),
    onSuccess: (meeting: Meeting) =>
      qc.setQueryData(qk.meeting(meeting.id), meeting),
  });
}
