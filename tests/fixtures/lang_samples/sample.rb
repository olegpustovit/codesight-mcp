# Serialization helpers.
module Serializable
  def serialize
    {}
  end
end

# Represents a user.
class User
  include Serializable

  def initialize(name, email)
    @name = name
    @email = email
  end

  # Finds a user by ID.
  def self.find(id)
    nil
  end

  def greet
    'greet'
  end

  private

  def valid_email?
    @email.include?('@')
  end
end

# Top-level helper.
def format_name(first, last)
  [first, last].join(" ")
end
