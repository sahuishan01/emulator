//! Original HOS binder parcels and the application layer's buffer queue.
//!
//! `IHOSBinderDriver::TransactParcel` carries an Android-style parcel: a
//! 16-byte header `{data_size, data_offset, objects_size, objects_offset}`
//! followed by little-endian 4-byte-aligned data. Producer transactions start
//! with an interface token (strict-mode policy + UTF-16 name).
//!
//! The queue has no asynchronous consumer: a queued buffer is presented and
//! released at queue time, so a slot is always free again for the next
//! dequeue. Transactions this module does not implement fail explicitly.

/// Android status codes carried in reply parcels.
pub const STATUS_OK: i32 = 0;
pub const STATUS_BAD_VALUE: i32 = -22;
pub const STATUS_NO_INIT: i32 = -19;
pub const STATUS_INVALID_OPERATION: i32 = -38;
/// `IGraphicBufferProducer::BUFFER_NEEDS_REALLOCATION` dequeue flag.
pub const BUFFER_NEEDS_REALLOCATION: i32 = 1;

pub const PRODUCER_INTERFACE: &str = "android.gui.IGraphicBufferProducer";
pub const NUM_BUFFER_SLOTS: usize = 64;
const PARCEL_HEADER: usize = 16;
/// Flattened Nintendo multi-fence: `{num_fences, 4 x {syncpoint id, value}}`.
const MULTI_FENCE_SIZE: usize = 0x24;
/// `GraphicBuffer` flatten magic, ASCII "GBFR" read as a big-endian word.
const GRAPHIC_BUFFER_MAGIC: u32 = 0x4742_4652;

/// Producer transaction codes.
pub mod code {
    pub const REQUEST_BUFFER: u32 = 1;
    pub const SET_BUFFER_COUNT: u32 = 2;
    pub const DEQUEUE_BUFFER: u32 = 3;
    pub const QUEUE_BUFFER: u32 = 7;
    pub const CANCEL_BUFFER: u32 = 8;
    pub const QUERY: u32 = 9;
    pub const CONNECT: u32 = 10;
    pub const DISCONNECT: u32 = 11;
    pub const SET_PREALLOCATED_BUFFER: u32 = 14;
}

/// `NATIVE_WINDOW_*` query keys.
pub mod query {
    pub const WIDTH: i32 = 0;
    pub const HEIGHT: i32 = 1;
    pub const FORMAT: i32 = 2;
    pub const MIN_UNDEQUEUED_BUFFERS: i32 = 3;
    pub const DEFAULT_WIDTH: i32 = 6;
    pub const DEFAULT_HEIGHT: i32 = 7;
    pub const TRANSFORM_HINT: i32 = 8;
    pub const CONSUMER_RUNNING_BEHIND: i32 = 9;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BinderError {
    /// The parcel header or payload is malformed.
    MalformedParcel,
    /// The interface token does not name the producer interface.
    WrongInterface,
    /// A transaction code this queue does not implement.
    UnsupportedTransaction(u32),
    /// The reply does not fit the caller's buffer.
    ReplyTooLarge,
}

/// Parsed view of a parcel's data section.
pub struct ParcelReader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> ParcelReader<'a> {
    pub fn new(parcel: &'a [u8]) -> Result<Self, BinderError> {
        let word = |at: usize| -> Result<usize, BinderError> {
            parcel
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")) as usize)
                .ok_or(BinderError::MalformedParcel)
        };
        let (size, offset) = (word(0)?, word(4)?);
        let end = offset.checked_add(size).ok_or(BinderError::MalformedParcel)?;
        if offset < PARCEL_HEADER || end > parcel.len() {
            return Err(BinderError::MalformedParcel);
        }
        Ok(Self {
            data: &parcel[offset..end],
            at: 0,
        })
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8], BinderError> {
        let end = self.at.checked_add(length).ok_or(BinderError::MalformedParcel)?;
        let bytes = self.data.get(self.at..end).ok_or(BinderError::MalformedParcel)?;
        self.at = end.next_multiple_of(4);
        Ok(bytes)
    }
    pub fn i32(&mut self) -> Result<i32, BinderError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }
    pub fn u32(&mut self) -> Result<u32, BinderError> {
        Ok(self.i32()? as u32)
    }
    /// Strict-mode policy word followed by a UTF-16 interface name.
    pub fn interface_token(&mut self) -> Result<String, BinderError> {
        self.i32()?;
        let length = usize::try_from(self.i32()?).map_err(|_| BinderError::MalformedParcel)?;
        let bytes = self.take((length + 1) * 2)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if units[length] != 0 {
            return Err(BinderError::MalformedParcel);
        }
        String::from_utf16(&units[..length]).map_err(|_| BinderError::MalformedParcel)
    }
    /// Flattenable: `{length, fd_count}` then `length` bytes (no fds on HOS).
    pub fn flattenable(&mut self) -> Result<&'a [u8], BinderError> {
        let length = usize::try_from(self.i32()?).map_err(|_| BinderError::MalformedParcel)?;
        if self.i32()? != 0 {
            return Err(BinderError::MalformedParcel);
        }
        self.take(length)
    }
}

/// Builds a reply parcel.
#[derive(Default)]
pub struct ParcelWriter {
    data: Vec<u8>,
}

impl ParcelWriter {
    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.data.extend_from_slice(&value.to_le_bytes());
        self
    }
    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.i32(value as i32)
    }
    pub fn flattenable(&mut self, bytes: &[u8]) -> &mut Self {
        self.i32(bytes.len() as i32).i32(0);
        self.data.extend_from_slice(bytes);
        self.data.resize(self.data.len().next_multiple_of(4), 0);
        self
    }
    pub fn finish(&self) -> Vec<u8> {
        let size = self.data.len() as u32;
        let mut parcel = Vec::with_capacity(PARCEL_HEADER + self.data.len());
        for word in [size, PARCEL_HEADER as u32, 0, PARCEL_HEADER as u32 + size] {
            parcel.extend_from_slice(&word.to_le_bytes());
        }
        parcel.extend_from_slice(&self.data);
        parcel
    }
}

/// Builds a request parcel with the producer interface token.
pub fn producer_request(body: impl FnOnce(&mut ParcelWriter)) -> Vec<u8> {
    let mut writer = ParcelWriter::default();
    writer.i32(0x100).i32(PRODUCER_INTERFACE.len() as i32);
    let mut name: Vec<u8> = PRODUCER_INTERFACE
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    name.resize(name.len().next_multiple_of(4), 0);
    writer.data.extend_from_slice(&name);
    body(&mut writer);
    writer.finish()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlotState {
    #[default]
    Free,
    Dequeued,
}

/// A preallocated buffer as flattened by the guest; geometry is decoded
/// when the flatten magic matches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphicBuffer {
    pub raw: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub usage: u32,
}

impl GraphicBuffer {
    fn parse(raw: &[u8]) -> Self {
        let word = |i: usize| {
            raw.get(i * 4..i * 4 + 4)
                .map_or(0, |b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
        };
        let known = word(0) == GRAPHIC_BUFFER_MAGIC;
        let field = |i| if known { word(i) } else { 0 };
        Self {
            raw: raw.to_vec(),
            width: field(1),
            height: field(2),
            stride: field(3),
            format: field(4),
            usage: field(5),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Slot {
    pub state: SlotState,
    pub buffer: Option<GraphicBuffer>,
    /// Whether the producer already fetched this buffer via REQUEST_BUFFER.
    pub requested: bool,
}

/// A frame handed to presentation when the producer queues a buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFrame {
    pub slot: usize,
    pub buffer: GraphicBuffer,
    pub frame_number: u64,
}

#[derive(Clone, Debug)]
pub struct BufferQueue {
    pub slots: Vec<Slot>,
    pub connected_api: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub frames_queued: u64,
}

impl BufferQueue {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            slots: vec![Slot::default(); NUM_BUFFER_SLOTS],
            connected_api: None,
            width,
            height,
            frames_queued: 0,
        }
    }

    fn slot_index(&self, slot: i32) -> Option<usize> {
        usize::try_from(slot).ok().filter(|s| *s < self.slots.len())
    }

    /// `{width, height, transform_hint, num_pending_buffers}` written raw.
    fn queue_output(&self, reply: &mut ParcelWriter) {
        reply.u32(self.width).u32(self.height).u32(0).u32(0);
    }

    /// Runs one producer transaction. `present` receives queued frames.
    pub fn transact(
        &mut self,
        code: u32,
        parcel: &[u8],
        mut present: impl FnMut(&QueuedFrame),
    ) -> Result<Vec<u8>, BinderError> {
        let mut input = ParcelReader::new(parcel)?;
        if input.interface_token()? != PRODUCER_INTERFACE {
            return Err(BinderError::WrongInterface);
        }
        let mut reply = ParcelWriter::default();
        match code {
            code::CONNECT => {
                if input.i32()? != 0 {
                    // Producer listeners are binder objects we cannot host.
                    reply.i32(STATUS_INVALID_OPERATION);
                    return Ok(reply.finish());
                }
                let api = input.i32()?;
                let _producer_controlled_by_app = input.i32()?;
                let status = if self.connected_api.is_some() || api <= 0 {
                    STATUS_BAD_VALUE
                } else {
                    self.connected_api = Some(api);
                    STATUS_OK
                };
                self.queue_output(&mut reply);
                reply.i32(status);
            }
            code::DISCONNECT => {
                let api = input.i32()?;
                let status = if self.connected_api == Some(api) {
                    self.connected_api = None;
                    for slot in &mut self.slots {
                        slot.state = SlotState::Free;
                    }
                    STATUS_OK
                } else {
                    STATUS_BAD_VALUE
                };
                reply.i32(status);
            }
            code::SET_PREALLOCATED_BUFFER => {
                let slot = input.i32()?;
                let buffer = if input.i32()? != 0 {
                    Some(GraphicBuffer::parse(input.flattenable()?))
                } else {
                    None
                };
                let status = match self.slot_index(slot) {
                    Some(index) if self.slots[index].state == SlotState::Free => {
                        self.slots[index] = Slot {
                            state: SlotState::Free,
                            buffer,
                            requested: false,
                        };
                        STATUS_OK
                    }
                    _ => STATUS_BAD_VALUE,
                };
                reply.i32(status);
            }
            code::SET_BUFFER_COUNT => {
                let count = input.i32()?;
                let busy = self.slots.iter().any(|s| s.state == SlotState::Dequeued);
                let status = if busy || !(0..=NUM_BUFFER_SLOTS as i32).contains(&count) {
                    STATUS_BAD_VALUE
                } else {
                    STATUS_OK
                };
                reply.i32(status);
            }
            code::DEQUEUE_BUFFER => {
                let _is_async = input.i32()?;
                let (width, height) = (input.u32()?, input.u32()?);
                let (_format, _usage) = (input.u32()?, input.u32()?);
                if self.connected_api.is_none() {
                    reply.i32(-1).i32(0).i32(STATUS_NO_INIT);
                    return Ok(reply.finish());
                }
                let free = self
                    .slots
                    .iter()
                    .position(|s| s.state == SlotState::Free && s.buffer.is_some());
                let Some(index) = free else {
                    reply.i32(-1).i32(0).i32(STATUS_INVALID_OPERATION);
                    return Ok(reply.finish());
                };
                let slot = &mut self.slots[index];
                slot.state = SlotState::Dequeued;
                let buffer = slot.buffer.as_ref().expect("checked");
                let size_mismatch = (width != 0 && width != buffer.width)
                    || (height != 0 && height != buffer.height);
                let mut status = STATUS_OK;
                if !slot.requested || size_mismatch {
                    status |= BUFFER_NEEDS_REALLOCATION;
                }
                reply
                    .i32(index as i32)
                    .i32(1)
                    .flattenable(&[0; MULTI_FENCE_SIZE])
                    .i32(status);
            }
            code::REQUEST_BUFFER => {
                let slot = input.i32()?;
                match self.slot_index(slot) {
                    Some(index)
                        if self.slots[index].state == SlotState::Dequeued
                            && self.slots[index].buffer.is_some() =>
                    {
                        self.slots[index].requested = true;
                        let raw = self.slots[index].buffer.as_ref().expect("checked").raw.clone();
                        reply.i32(1).flattenable(&raw).i32(STATUS_OK);
                    }
                    _ => {
                        reply.i32(0).i32(STATUS_BAD_VALUE);
                    }
                }
            }
            code::QUEUE_BUFFER => {
                let slot = input.i32()?;
                let _queue_input = input.flattenable()?;
                match self.slot_index(slot) {
                    Some(index) if self.slots[index].state == SlotState::Dequeued => {
                        self.frames_queued += 1;
                        let frame = QueuedFrame {
                            slot: index,
                            buffer: self.slots[index].buffer.clone().expect("dequeued slot has a buffer"),
                            frame_number: self.frames_queued,
                        };
                        present(&frame);
                        // Presented synchronously, then released back to the producer.
                        self.slots[index].state = SlotState::Free;
                        self.queue_output(&mut reply);
                        reply.i32(STATUS_OK);
                    }
                    _ => {
                        self.queue_output(&mut reply);
                        reply.i32(STATUS_BAD_VALUE);
                    }
                }
            }
            code::CANCEL_BUFFER => {
                let slot = input.i32()?;
                let _fence = input.flattenable()?;
                let status = match self.slot_index(slot) {
                    Some(index) if self.slots[index].state == SlotState::Dequeued => {
                        self.slots[index].state = SlotState::Free;
                        STATUS_OK
                    }
                    _ => STATUS_BAD_VALUE,
                };
                reply.i32(status);
            }
            code::QUERY => {
                let what = input.i32()?;
                let value = match what {
                    query::WIDTH | query::DEFAULT_WIDTH => Some(self.width as i32),
                    query::HEIGHT | query::DEFAULT_HEIGHT => Some(self.height as i32),
                    query::FORMAT => Some(1), // RGBA_8888
                    query::MIN_UNDEQUEUED_BUFFERS => Some(1),
                    query::TRANSFORM_HINT | query::CONSUMER_RUNNING_BEHIND => Some(0),
                    _ => None,
                };
                match value {
                    Some(value) => reply.i32(value).i32(STATUS_OK),
                    None => reply.i32(0).i32(STATUS_BAD_VALUE),
                };
            }
            other => return Err(BinderError::UnsupportedTransaction(other)),
        }
        Ok(reply.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_words(parcel: &[u8]) -> Vec<i32> {
        let mut reader = ParcelReader::new(parcel).unwrap();
        let mut words = Vec::new();
        while let Ok(word) = reader.i32() {
            words.push(word);
        }
        words
    }

    fn graphic_buffer(width: u32, height: u32) -> Vec<u8> {
        let mut raw = Vec::new();
        for word in [GRAPHIC_BUFFER_MAGIC, width, height, width, 1, 0xb00, 0x55] {
            raw.extend_from_slice(&word.to_le_bytes());
        }
        raw
    }

    fn connected() -> BufferQueue {
        let mut queue = BufferQueue::new(1280, 720);
        let reply = queue
            .transact(code::CONNECT, &producer_request(|w| { w.i32(0).i32(2).i32(0); }), |_| {})
            .unwrap();
        assert_eq!(read_words(&reply), [1280, 720, 0, 0, STATUS_OK]);
        queue
    }

    fn dequeue(queue: &mut BufferQueue) -> Vec<i32> {
        let request = producer_request(|w| {
            w.i32(0).u32(1280).u32(720).u32(1).u32(0xb00);
        });
        read_words(&queue.transact(code::DEQUEUE_BUFFER, &request, |_| {}).unwrap())
    }

    #[test]
    fn preallocated_buffers_cycle_through_dequeue_request_queue() {
        let mut queue = connected();
        for slot in 0..2 {
            let request = producer_request(|w| {
                w.i32(slot).i32(1).flattenable(&graphic_buffer(1280, 720));
            });
            let reply = queue.transact(code::SET_PREALLOCATED_BUFFER, &request, |_| {}).unwrap();
            assert_eq!(read_words(&reply), [STATUS_OK]);
        }
        let words = dequeue(&mut queue);
        // slot, fence present, fence {len, fds, 9 words}, status.
        assert_eq!(words[0], 0);
        assert_eq!(words[1..4], [1, MULTI_FENCE_SIZE as i32, 0]);
        assert_eq!(*words.last().unwrap(), BUFFER_NEEDS_REALLOCATION);

        let reply = queue
            .transact(code::REQUEST_BUFFER, &producer_request(|w| { w.i32(0); }), |_| {})
            .unwrap();
        let words = read_words(&reply);
        assert_eq!(words[..3], [1, 28, 0]);
        assert_eq!(words[4..6], [1280, 720]);
        assert_eq!(*words.last().unwrap(), STATUS_OK);

        let mut presented = Vec::new();
        let request = producer_request(|w| {
            w.i32(0).flattenable(&[0; 0x54]);
        });
        let reply = queue
            .transact(code::QUEUE_BUFFER, &request, |f| presented.push(f.clone()))
            .unwrap();
        assert_eq!(read_words(&reply), [1280, 720, 0, 0, STATUS_OK]);
        assert_eq!(presented.len(), 1);
        assert_eq!((presented[0].slot, presented[0].buffer.width), (0, 1280));

        // Slot 0 was released at present time and is now already requested.
        let words = dequeue(&mut queue);
        assert_eq!((words[0], *words.last().unwrap()), (0, STATUS_OK));
    }

    #[test]
    fn rejections_leave_state_unchanged() {
        let mut queue = BufferQueue::new(1280, 720);
        // Not connected: dequeue reports NO_INIT.
        assert_eq!(*dequeue(&mut queue).last().unwrap(), STATUS_NO_INIT);
        let mut queue = connected();
        // Second connect is refused.
        let reply = queue
            .transact(code::CONNECT, &producer_request(|w| { w.i32(0).i32(2).i32(0); }), |_| {})
            .unwrap();
        assert_eq!(*read_words(&reply).last().unwrap(), STATUS_BAD_VALUE);
        // No buffers yet: dequeue fails, no slot changes state.
        assert_eq!(*dequeue(&mut queue).last().unwrap(), STATUS_INVALID_OPERATION);
        assert!(queue.slots.iter().all(|s| s.state == SlotState::Free));
        // Queue of a slot that was never dequeued.
        let request = producer_request(|w| {
            w.i32(3).flattenable(&[0; 0x54]);
        });
        let reply = queue.transact(code::QUEUE_BUFFER, &request, |_| panic!("no present")).unwrap();
        assert_eq!(*read_words(&reply).last().unwrap(), STATUS_BAD_VALUE);
        // Out-of-range slot.
        let request = producer_request(|w| {
            w.i32(64).i32(1).flattenable(&graphic_buffer(1, 1));
        });
        let reply = queue.transact(code::SET_PREALLOCATED_BUFFER, &request, |_| {}).unwrap();
        assert_eq!(read_words(&reply), [STATUS_BAD_VALUE]);
        // Unknown transaction codes and wrong interfaces fail explicitly.
        assert_eq!(
            queue.transact(6, &producer_request(|_| {}), |_| {}),
            Err(BinderError::UnsupportedTransaction(6))
        );
        let mut foreign = ParcelWriter::default();
        foreign.i32(0).i32(1).i32(0x41);
        assert_eq!(
            queue.transact(code::QUERY, &foreign.finish(), |_| {}),
            Err(BinderError::WrongInterface)
        );
        assert_eq!(
            queue.transact(code::QUERY, &[0; 8], |_| {}),
            Err(BinderError::MalformedParcel)
        );
        // Disconnect with the wrong API keeps the connection.
        let reply = queue
            .transact(code::DISCONNECT, &producer_request(|w| { w.i32(1); }), |_| {})
            .unwrap();
        assert_eq!(read_words(&reply), [STATUS_BAD_VALUE]);
        assert_eq!(queue.connected_api, Some(2));
    }

    #[test]
    fn query_reports_geometry_and_rejects_unknown_keys() {
        let mut queue = connected();
        let ask = |queue: &mut BufferQueue, what| {
            read_words(&queue.transact(code::QUERY, &producer_request(|w| { w.i32(what); }), |_| {}).unwrap())
        };
        assert_eq!(ask(&mut queue, query::WIDTH), [1280, STATUS_OK]);
        assert_eq!(ask(&mut queue, query::DEFAULT_HEIGHT), [720, STATUS_OK]);
        assert_eq!(ask(&mut queue, 1234), [0, STATUS_BAD_VALUE]);
    }
}
