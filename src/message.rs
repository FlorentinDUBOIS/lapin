use crate::{
    acker::Acker,
    internal_rpc::InternalRPCHandle,
    protocol::AMQPError,
    types::{LongLongUInt, LongUInt, ShortString, ShortUInt},
    BasicProperties, Channel, Result,
};
use std::ops::Deref;

/// Type wrapping the output of a consumer
///
/// - Ok(Some((channel, delivery))) carries the delivery alongside its channel
/// - Ok(None) means that the consumer got canceled
/// - Err(error) carries the error and is always followed by Ok(None)
pub type DeliveryResult = Result<Option<(Channel, Delivery)>>;

/// A received AMQP message.
///
/// The message has to be acknowledged after processing by calling
/// [`Acker::ack`], [`Acker::nack`] or [`Acker::reject`].
/// (Multiple acknowledgments are also possible).
///
/// [`Acker::ack`]: ../struct.Acker.html#method.ack
/// [`Acker::nack`]: ../struct.Acker.html#method.nack
/// [`Acker::reject`]: ../struct.Acker.html#method.reject
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    /// The delivery tag of the message. Use this for
    /// acknowledging the message.
    pub delivery_tag: LongLongUInt,

    /// The exchange of the message. May be an empty string
    /// if the default exchange is used.
    pub exchange: ShortString,

    /// The routing key of the message. May be an empty string
    /// if no routing key is specified.
    pub routing_key: ShortString,

    /// Whether this message was redelivered
    pub redelivered: bool,

    /// Contains the properties and the headers of the
    /// message.
    pub properties: BasicProperties,

    /// The payload of the message in binary format.
    pub data: Vec<u8>,

    /// The acker used to ack/nack the message
    pub acker: Acker,
}

impl Delivery {
    pub(crate) fn new(
        channel_id: u16,
        delivery_tag: LongLongUInt,
        exchange: ShortString,
        routing_key: ShortString,
        redelivered: bool,
        internal_rpc: Option<InternalRPCHandle>,
    ) -> Self {
        Self {
            delivery_tag,
            exchange,
            routing_key,
            redelivered,
            properties: BasicProperties::default(),
            data: Vec::default(),
            acker: Acker::new(channel_id, delivery_tag, internal_rpc),
        }
    }

    pub(crate) fn receive_content(&mut self, data: Vec<u8>) {
        self.data.extend(data);
    }
}

impl Deref for Delivery {
    type Target = Acker;

    fn deref(&self) -> &Self::Target {
        &self.acker
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BasicGetMessage {
    pub delivery: Delivery,
    pub message_count: LongUInt,
}

impl BasicGetMessage {
    pub(crate) fn new(
        channel_id: u16,
        delivery_tag: LongLongUInt,
        exchange: ShortString,
        routing_key: ShortString,
        redelivered: bool,
        message_count: LongUInt,
        internal_rpc: InternalRPCHandle,
    ) -> Self {
        Self {
            delivery: Delivery::new(
                channel_id,
                delivery_tag,
                exchange,
                routing_key,
                redelivered,
                Some(internal_rpc),
            ),
            message_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BasicReturnMessage {
    pub delivery: Delivery,
    pub reply_code: ShortUInt,
    pub reply_text: ShortString,
}

impl BasicReturnMessage {
    pub(crate) fn new(
        exchange: ShortString,
        routing_key: ShortString,
        reply_code: ShortUInt,
        reply_text: ShortString,
    ) -> Self {
        Self {
            delivery: Delivery::new(0, 0, exchange, routing_key, false, None),
            reply_code,
            reply_text,
        }
    }

    pub fn error(&self) -> Option<AMQPError> {
        AMQPError::from_id(self.reply_code, self.reply_text.clone())
    }
}
