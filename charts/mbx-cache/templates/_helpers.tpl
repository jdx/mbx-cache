{{/*
Resource name for this release.
*/}}
{{- define "mbx-cache.fullname" -}}
{{- printf "%s-mbx-cache" .Release.Name -}}
{{- end -}}

{{/*
ServiceAccount the pods run as.

When the chart does not create one, fall back to the namespace's `default`
rather than the generated name: referencing a ServiceAccount that nothing
created leaves the pods unschedulable.
*/}}
{{- define "mbx-cache.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "mbx-cache.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}
