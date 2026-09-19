use wasmtime::component::StreamReader;

use crate::stream_input::BufferProducer;
use crate::{RelaySink, RelaySource};

use super::{Runtime, RuntimeError, StreamGroupInput, StreamMetrics, StreamResult};

impl Runtime {
    pub async fn relay_stream_prefix(
        &self,
        workflow: &kairo_core::Workflow,
        input: &std::path::Path,
        consumer_start: usize,
        sink: RelaySink,
    ) -> crate::Result<()> {
        self.relay_stream_group_prefix(
            workflow,
            0,
            StreamGroupInput::File(input),
            consumer_start,
            sink,
        )
        .await
    }

    pub async fn relay_stream_group_prefix(
        &self,
        workflow: &kairo_core::Workflow,
        start_index: usize,
        input: StreamGroupInput<'_>,
        consumer_start: usize,
        sink: RelaySink,
    ) -> crate::Result<()> {
        let prepared = self.prepare_stream_workflow(workflow)?;
        if consumer_start <= start_index || consumer_start > prepared.transforms.len() {
            return Err(RuntimeError::InvalidStreamWorkflowInput);
        }
        let mut store = self.new_store(workflow.resources())?;
        let mut stream = match input {
            StreamGroupInput::File(path) => {
                let (source, _, _) = super::StreamInput::open(
                    path,
                    self.config.max_stream_input_bytes,
                    self.config.stream_chunk_bytes,
                    false,
                )?;
                source.reader(&mut store)?
            }
            StreamGroupInput::Bytes(bytes) => StreamReader::new(
                &mut store,
                BufferProducer::from_bytes(bytes, self.config.stream_chunk_bytes),
            )
            .map_err(|source| RuntimeError::CreateStream { source })?,
        };
        for transform in &prepared.transforms[start_index..consumer_start] {
            let instance = transform
                .transform
                .instantiate_async(&mut store)
                .await
                .map_err(|error| {
                    self.stream_step_error(&transform.name, self.instantiation_error(error, &store))
                })?;
            let call = store
                .run_concurrent(async |access| instance.call_transform(access, stream).await)
                .await;
            stream = self.finish_stream_call(call, &store, &transform.name)?;
        }
        let result = store
            .run_concurrent(async |access| {
                access
                    .with(|store| stream.pipe(store, sink.component_consumer()))
                    .map_err(|source| RuntimeError::RelayPipe { source })?;
                sink.wait_closed()
                    .await
                    .map_err(|message| RuntimeError::RelayClosed { message })
            })
            .await
            .map_err(|source| RuntimeError::RelayConcurrent { source })?;
        result?;
        Ok(())
    }

    pub async fn consume_relay_suffix(
        &self,
        workflow: &kairo_core::Workflow,
        consumer_start: usize,
        source: RelaySource,
    ) -> crate::Result<StreamResult> {
        let prepared = self.prepare_stream_workflow(workflow)?;
        if consumer_start == 0 || consumer_start > prepared.transforms.len() {
            return Err(RuntimeError::InvalidStreamWorkflowInput);
        }
        let mut store = self.new_store(workflow.resources())?;
        let mut stream = StreamReader::new(&mut store, source.component_producer())
            .map_err(|source| RuntimeError::CreateStream { source })?;
        for transform in &prepared.transforms[consumer_start..] {
            let instance = transform
                .transform
                .instantiate_async(&mut store)
                .await
                .map_err(|error| {
                    self.stream_step_error(&transform.name, self.instantiation_error(error, &store))
                })?;
            let call = store
                .run_concurrent(async |access| instance.call_transform(access, stream).await)
                .await;
            stream = self.finish_stream_call(call, &store, &transform.name)?;
        }
        let (summary, values, outputs) = self
            .run_stream_terminal(workflow, &prepared, stream, &mut store, None)
            .await?;
        Ok(StreamResult {
            bytes: summary >> 32,
            checksum: summary as u32,
            duration: std::time::Duration::ZERO,
            metrics: StreamMetrics::default(),
            input_hash: String::new(),
            values,
            outputs,
        })
    }
}
