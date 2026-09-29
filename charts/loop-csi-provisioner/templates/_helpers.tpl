{{/* The CSI driver name is fixed by the driver's Identity API. */}}
{{- define "loop-csi-provisioner.driverName" -}}
loop-csi-provisioner
{{- end }}

{{- define "loop-csi-provisioner.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "loop-csi-provisioner.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "loop-csi-provisioner.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "loop-csi-provisioner.labels" -}}
helm.sh/chart: {{ include "loop-csi-provisioner.chart" . }}
app.kubernetes.io/name: {{ include "loop-csi-provisioner.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/* Usage: include "loop-csi-provisioner.selectorLabels" (dict "context" $ "component" "controller") */}}
{{- define "loop-csi-provisioner.selectorLabels" -}}
app.kubernetes.io/name: {{ include "loop-csi-provisioner.name" .context }}
app.kubernetes.io/instance: {{ .context.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end }}

{{- define "loop-csi-provisioner.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (printf "%s-controller" (include "loop-csi-provisioner.fullname" .)) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "loop-csi-provisioner.image" -}}
{{- printf "%s:%s" .Values.image.repository (default (printf "v%s" .Chart.AppVersion) .Values.image.tag) }}
{{- end }}

{{/* Usage: include "loop-csi-provisioner.sidecarImage" .Values.sidecars.provisioner */}}
{{- define "loop-csi-provisioner.sidecarImage" -}}
{{- printf "%s:%s" .image.repository .image.tag }}
{{- end }}

{{/* Arguments shared by the controller and node driver containers. */}}
{{- define "loop-csi-provisioner.driverArgs" -}}
- --listen=unix:///csi/csi.sock
- --base-directory=/var/lib/loop-csi-provisioner
{{- if not .Values.allowedUrlPrefixes }}
{{- fail "allowedUrlPrefixes must list the storage URLs the driver may mount, e.g. --set allowedUrlPrefixes={nfs://nfs.example.com/export}" }}
{{- end }}
{{- range .Values.allowedUrlPrefixes }}
- --allowed-url-prefix={{ . }}
{{- end }}
{{- if not (has .Values.log.format (list "json" "text")) }}
{{- fail "log.format must be json or text" }}
{{- end }}
{{- if eq .Values.log.format "text" }}
- --plaintext-log
{{- end }}
{{- end }}

{{- define "loop-csi-provisioner.driverEnv" -}}
- name: RUST_LOG
  value: {{ .Values.log.level | quote }}
{{- end }}
