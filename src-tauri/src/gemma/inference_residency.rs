use super::*;

impl GemmaService {
    pub fn infer_model_sync(
        &self,
        model_id: &str,
        request: InferRequest,
    ) -> Result<InferResponse, GemmaError> {
        self.infer_model_with_stream_sync(model_id, request, None)
    }

    pub fn infer_model_with_stream_sync(
        &self,
        model_id: &str,
        request: InferRequest,
        stream: Option<&mut dyn LocalGenerationStream>,
    ) -> Result<InferResponse, GemmaError> {
        if deterministic_transform::can_execute_without_model(&request) {
            return self.infer_with_stream_sync(request, stream);
        }
        let model_id = model_id.trim();
        if let Some(classifier_lane) = self.classifier_lane_for_model(model_id) {
            return classifier_lane.infer_with_stream_sync(request, stream);
        }
        if !model_id.is_empty() {
            self.ensure_model_loaded(model_id, request.context_size)?;
        }
        self.infer_with_stream_sync(request, stream)
    }

    pub(super) fn ensure_model_loaded(
        &self,
        model_id: &str,
        min_context_size: Option<u32>,
    ) -> Result<(), GemmaError> {
        if self.classifier_lane_for_model(model_id).is_some() {
            return Ok(());
        }
        let _load_guard = self.lock_model_load();
        let min_context_size = min_context_size.map(|size| size.clamp(512, 131_072));
        let desired_model_dir = local_model_dir(model_id)?;
        let manifest = inspect_local_model_directory(&desired_model_dir, model_id)?;
        if !is_ready_gguf(&manifest) {
            return Err(GemmaError {
                code: "local_model_incompatible",
                message: manifest.compatibility_message,
            });
        }
        let previous_model = {
            let mut state = self.lock_state();
            if state.model.as_ref().is_some_and(|model| {
                model.model_dir == desired_model_dir
                    && min_context_size.is_none_or(|required| {
                        model.profile.runtime_config.context_size >= required
                    })
            }) {
                return Ok(());
            }
            state.status = GemmaStatus::Loading;
            state.degraded_reason = None;
            state.model.take()
        };
        drop(previous_model);
        self.load_model_from_dir_with_context_locked(desired_model_dir, min_context_size)
            .inspect_err(|error| self.enter_local_generation_degraded(error.clone()))
    }
}
