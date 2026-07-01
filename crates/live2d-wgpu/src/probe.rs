use crate::*;

#[cfg(feature = "probe")]
pub(crate) struct GpuTimestampFrame {
    pub(crate) query_set: wgpu::QuerySet,
    pub(crate) resolve_buffer: wgpu::Buffer,
    pub(crate) readback_buffer: wgpu::Buffer,
    pub(crate) query_count: u32,
    pub(crate) mask_indices: Option<(u32, u32)>,
    pub(crate) main_indices: Option<(u32, u32)>,
}

#[cfg(feature = "probe")]
impl GpuTimestampFrame {
    pub(crate) fn new(
        device: &wgpu::Device,
        include_mask: bool,
        include_main: bool,
    ) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let mut next = 0;
        let mask_indices = include_mask.then(|| {
            let indices = (next, next + 1);
            next += 2;
            indices
        });
        let main_indices = include_main.then(|| {
            let indices = (next, next + 1);
            next += 2;
            indices
        });
        let query_count = next;
        if query_count == 0 {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Live2D Probe Timestamp Query Set"),
            ty: wgpu::QueryType::Timestamp,
            count: query_count,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Probe Timestamp Resolve"),
            size: query_count as u64 * std::mem::size_of::<u64>() as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Probe Timestamp Readback"),
            size: query_count as u64 * std::mem::size_of::<u64>() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Self {
            query_set,
            resolve_buffer,
            readback_buffer,
            query_count,
            mask_indices,
            main_indices,
        })
    }

    pub(crate) fn timestamp_writes(
        &self,
        indices: (u32, u32),
    ) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(indices.0),
            end_of_pass_write_index: Some(indices.1),
        }
    }

    pub(crate) fn mask_timestamp_writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.mask_indices
            .map(|indices| self.timestamp_writes(indices))
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let byte_count = self.query_count as u64 * std::mem::size_of::<u64>() as u64;
        encoder.resolve_query_set(
            &self.query_set,
            0..self.query_count,
            &self.resolve_buffer,
            0,
        );
        encoder.copy_buffer_to_buffer(
            &self.resolve_buffer,
            0,
            &self.readback_buffer,
            0,
            byte_count,
        );
    }
}

pub(crate) fn read_timestamp_values(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    query_count: u32,
) -> Result<Vec<u64>, String> {
    let byte_count = query_count as u64 * std::mem::size_of::<u64>() as u64;
    let values = {
        let slice = buffer.slice(0..byte_count);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()
            .map_err(|err| format!("failed to receive timestamp map result: {err}"))?
            .map_err(|err| format!("failed to map timestamp buffer: {err}"))?;
        let data = slice.get_mapped_range();
        data.chunks_exact(std::mem::size_of::<u64>())
            .map(|chunk| u64::from_ne_bytes(chunk.try_into().expect("u64 timestamp chunk")))
            .collect::<Vec<_>>()
    };
    buffer.unmap();
    Ok(values)
}

#[cfg(feature = "probe")]
pub(crate) fn record_gpu_pass_nanos<P>(
    probe: &P,
    stage: Stage,
    pass: &'static str,
    values: &[u64],
    indices: (u32, u32),
    timestamp_period: f64,
) where
    P: ProbeSink,
{
    let Some(start) = values.get(indices.0 as usize) else {
        return;
    };
    let Some(end) = values.get(indices.1 as usize) else {
        return;
    };
    let nanos = ((*end).saturating_sub(*start) as f64 * timestamp_period)
        .round()
        .min(u64::MAX as f64) as u64;
    counter(
        probe,
        stage,
        "gpu_pass_nanos",
        nanos,
        vec![ProbeAttr::new("pass", pass)],
    );
}
